//! Variable write/read model powering UsedOnce/NeverUsed for PHP.
//!
//! Scope rules mirror the Rust backend: reads resolve through Block
//! scopes (compound statements, anonymous/arrow function bodies) but
//! stop at Function boundaries (functions and methods). PHP variables
//! keep their `$` in the source; names are stored and reported without
//! it.

use std::collections::HashMap;

use tree_sitter::Node;

use super::PhpFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// plain assignment / destructuring -- inline candidate
    Assign,
    /// compound assignment or catch binding -- never a candidate
    Binding,
}

#[derive(Clone, Copy, Debug)]
struct Write {
    byte: usize,
    node_id: usize,
    plain: bool,
    rhs: Option<usize>,
}

struct Entry {
    intro_byte: usize,
    intro_kind: IntroKind,
    writes: Vec<Write>,
    reads: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Root,
    Function,
    Block,
}

pub(super) struct Scope {
    parent: Option<usize>,
    kind: ScopeKind,
    entries: HashMap<Box<str>, Entry>,
}

/// Subtrees carrying no local-variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "formal_parameters",
    "property_declaration",
    "const_declaration",
    "namespace_definition",
    "namespace_use_declaration",
];

const VETO_KINDS: &[&str] = &[
    "if_statement",
    "while_statement",
    "do_statement",
    "for_statement",
    "foreach_statement",
    "switch_statement",
    "try_statement",
    "catch_clause",
    "match_expression",
];

/// Kinds that open a nested scope. PHP has no block scoping -- braces
/// are not scopes -- so only closures (which capture) get one; their
/// bodies resolve outward through the Block just like Rust closures.
const BLOCK_SCOPED: &[&str] = &["anonymous_function", "arrow_function"];

pub fn used_once_offenses(fm: &PhpFile) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if let Some(o) = single_use(fm, &nodes, name, e) {
                out.push(o);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

pub fn never_used_offenses(fm: &PhpFile) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if !e.reads.is_empty() || e.writes.is_empty() {
                continue;
            }
            let first = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
            let (line, column) = fm.line_col(first);
            out.push(NeverUsedOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

fn single_use<'t>(
    fm: &'t PhpFile<'t>,
    nodes: &HashMap<usize, Node<'t>>,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    if e.intro_kind != IntroKind::Assign || e.writes.len() != 1 || e.reads.len() != 1 {
        return None;
    }
    let w = &e.writes[0];
    if !w.plain || w.rhs.is_none() || e.reads[0] <= w.byte {
        return None;
    }
    let rhs = nodes.get(&w.rhs?)?;
    let write_node = nodes.get(&w.node_id)?;
    if !pure(*rhs) || !unconditionally_executed(*write_node) {
        return None;
    }
    let (line, column) = fm.line_col(w.byte);
    Some(UsedOnceOffense {
        line,
        column,
        name: name.to_string(),
    })
}

/// Conservative RHS purity: literals, arrays of literals, and operator
/// compositions over them. Interpolated strings reference variables and
/// are rejected via their children.
fn pure(n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "string" | "encapsed_string" => children_pure(n),
        "boolean" | "null" => true,
        "array_creation_expression" | "array_element_initializer"
        | "parenthesized_expression" | "binary_expression" | "unary_op_expression" => {
            children_pure(n)
        }
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

fn unconditionally_executed(write_node: Node) -> bool {
    const OWNERS: [&str; 2] = ["function_definition", "method_declaration"];
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if VETO_KINDS.contains(&n.kind()) {
            return false;
        }
        if OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

fn index_nodes<'t>(root: Node<'t>) -> HashMap<usize, Node<'t>> {
    let mut map = HashMap::new();
    rec(root, &mut map);
    map
}

fn rec<'t>(n: Node<'t>, map: &mut HashMap<usize, Node<'t>>) {
    map.insert(n.id(), n);
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        rec(child, map);
    }
}

// ---------------------------------------------------------------------
// Scope-model collection

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<Scope> {
    let mut c = Collector {
        src,
        scopes: vec![Scope {
            parent: None,
            kind: ScopeKind::Root,
            entries: HashMap::new(),
        }],
    };
    c.walk(root, 0);
    c.scopes
}

struct Collector<'a> {
    src: &'a [u8],
    scopes: Vec<Scope>,
}

impl Collector<'_> {
    /// PHP variable names keep `$` in the tree; store/report without it.
    fn var_name(&self, n: Node) -> String {
        let raw = n.utf8_text(self.src).unwrap_or("");
        raw.strip_prefix('$').unwrap_or(raw).to_string()
    }

    fn open_scope(&mut self, kind: ScopeKind, parent: usize) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let data = &self.scopes[scope];
        if let Some(e) = data.entries.get(name) {
            return if e.intro_byte <= pos { Some(scope) } else { None };
        }
        match data.kind {
            ScopeKind::Block => self.lookup(data.parent?, pos, name),
            _ => None,
        }
    }

    fn bind(&mut self, scope: usize, name: &str, w: Write, intro: IntroKind) {
        if name.starts_with('_') {
            return;
        }
        match self.lookup(scope, w.byte, name) {
            Some(s) => {
                self.scopes[s]
                    .entries
                    .get_mut(name)
                    .expect("looked-up entry")
                    .writes
                    .push(w);
            }
            None => {
                self.scopes[scope].entries.insert(
                    Box::from(name),
                    Entry {
                        intro_byte: w.byte,
                        intro_kind: intro,
                        writes: vec![w],
                        reads: Vec::new(),
                    },
                );
            }
        }
    }

    fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
        if name.starts_with('_') {
            return;
        }
        if let Some(s) = self.lookup(scope, byte, name) {
            self.scopes[s]
                .entries
                .get_mut(name)
                .expect("looked-up entry")
                .reads
                .push(byte);
        }
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        if BLOCK_SCOPED.contains(&kind) {
            let s = self.open_scope(ScopeKind::Block, scope);
            self.walk_children(n, s);
            return;
        }
        match kind {
            "function_definition" | "method_declaration" => {
                let s = self.open_scope(ScopeKind::Function, scope);
                self.walk_children(n, s);
            }
            "assignment_expression" => {
                let left = n.child_by_field_name("left");
                let right = n.child_by_field_name("right");
                if let Some(left) = left {
                    if left.kind() == "variable_name" {
                        let w = Write {
                            byte: left.start_byte(),
                            node_id: left.id(),
                            plain: true,
                            rhs: right.map(|r| r.id()),
                        };
                        let name = self.var_name(left);
                        self.bind(scope, &name, w, IntroKind::Assign);
                    } else if left.kind() == "list_literal" {
                        // [$a, $b] = ... : each element binds per name
                        let mut c = left.walk();
                        for el in left.children(&mut c) {
                            if el.kind() == "variable_name" {
                                let w = Write {
                                    byte: el.start_byte(),
                                    node_id: el.id(),
                                    plain: true,
                                    rhs: None,
                                };
                                let name = self.var_name(el);
                                self.bind(scope, &name, w, IntroKind::Assign);
                            } else if el.kind() != "," && el.kind() != "[" && el.kind() != "]" {
                                self.walk(el, scope);
                            }
                        }
                    } else {
                        // member/subscript target: operands are reads
                        self.walk_children(left, scope);
                    }
                }
                if let Some(right) = right {
                    self.walk(right, scope);
                }
            }
            "augmented_assignment_expression" => {
                if let Some(left) = n.child_by_field_name("left") {
                    if left.kind() == "variable_name" {
                        let byte = left.start_byte();
                        let w = Write {
                            byte,
                            node_id: left.id(),
                            plain: false,
                            rhs: None,
                        };
                        let name = self.var_name(left);
                        self.bind(scope, &name, w, IntroKind::Binding);
                        self.record_read(scope, &name, byte + 1);
                    } else {
                        self.walk_children(left, scope);
                    }
                }
                if let Some(right) = n.child_by_field_name("right") {
                    self.walk(right, scope);
                }
            }
            "foreach_statement" => {
                // the `as` head is loop protocol -- never tracked, like
                // Python for-targets and Go range heads
                let s = self.open_scope(ScopeKind::Block, scope);
                let skipped = child_of_kind(n, "pair").map(|p| p.id());
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if Some(child.id()) != skipped {
                        self.walk(child, s);
                    }
                }
            }
            "catch_clause" => {
                // `catch (E $e)`: the binding is protocol, tracked as a
                // non-candidate write like Python's except-as
                let s = self.open_scope(ScopeKind::Block, scope);
                let binder = child_of_kind(n, "variable_name");
                let skipped = binder.as_ref().map(|b| b.id());
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if Some(child.id()) == skipped {
                        continue;
                    }
                    self.walk(child, s);
                }
                if let Some(b) = binder {
                    let w = Write {
                        byte: b.start_byte(),
                        node_id: b.id(),
                        plain: false,
                        rhs: None,
                    };
                    let name = self.var_name(b);
                    self.bind(s, &name, w, IntroKind::Binding);
                }
            }
            "variable_name" => {
                let name = self.var_name(n);
                if name == "this" {
                    return;
                }
                self.record_read(scope, &name, n.start_byte());
            }
            _ => self.walk_children(n, scope),
        }
    }
}

fn child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).find(|ch| ch.kind() == kind)
}

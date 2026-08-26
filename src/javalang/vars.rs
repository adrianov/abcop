//! Variable write/read model powering UsedOnce/NeverUsed for Java.
//!
//! Scope rules mirror the Rust backend: reads resolve through Block
//! scopes (blocks, lambda bodies, switch blocks) but stop at Function
//! boundaries. Member names (`field_access`/`method_invocation` name
//! slots) and qualified type names are never variable reads.

use std::collections::HashMap;

use tree_sitter::Node;

use super::JavaFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// local declarator / plain assignment -- inline candidate
    Assign,
    /// compound assignment or catch/resource binding -- never a candidate
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
    "field_declaration",
    "import_declaration",
    "package_declaration",
    "scoped_identifier",
];

const VETO_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "enhanced_for_statement",
    "while_statement",
    "do_statement",
    "switch_expression",
    "switch_statement",
    "try_statement",
    "try_with_resources_statement",
    "catch_clause",
];

/// Kinds that open a nested scope: blocks and lambdas capture; Java's
/// switch groups live inside the enclosing function scope otherwise.
const BLOCK_SCOPED: &[&str] = &["block", "lambda_expression", "switch_block"];

pub fn used_once_offenses(fm: &JavaFile) -> Vec<UsedOnceOffense> {
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

pub fn never_used_offenses(fm: &JavaFile) -> Vec<NeverUsedOffense> {
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
    fm: &'t JavaFile<'t>,
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

/// Conservative RHS purity: literals and operator compositions over
/// them; references to other locals, calls and array creations are
/// rejected, mirroring the Rust backend.
fn pure(n: Node) -> bool {
    match n.kind() {
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "decimal_floating_point_literal"
        | "char_literal"
        | "string_literal"
        | "boolean_literal"
        | "null_literal"
        | "true"
        | "false" => true,
        "parenthesized_expression" | "binary_expression" | "unary_expression" => children_pure(n),
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
    const OWNERS: [&str; 2] = ["method_declaration", "constructor_declaration"];
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
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
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

    fn walk_children_excluding_fields(&mut self, n: Node, scope: usize, skip: &[&str]) {
        let skipped: Vec<_> = skip
            .iter()
            .filter_map(|f| n.child_by_field_name(f).map(|c| c.id()))
            .collect();
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if skipped.contains(&child.id()) {
                continue;
            }
            self.walk(child, scope);
        }
    }

    /// Bind every `@name` identifier of a variable_declarator.
    fn bind_declarator(&mut self, n: Node, scope: usize, allow_rhs: bool) {
        if let Some(name_node) = n.child_by_field_name("name") {
            if name_node.kind() == "identifier" {
                let w = Write {
                    byte: name_node.start_byte(),
                    node_id: name_node.id(),
                    plain: true,
                    rhs: if allow_rhs {
                        n.child_by_field_name("value").map(|v| v.id())
                    } else {
                        None
                    },
                };
                let name = self.text(name_node).to_string();
                self.bind(scope, &name, w, IntroKind::Assign);
            }
        }
    }

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        if BLOCK_SCOPED.contains(&kind) {
            let s = self.open_scope(ScopeKind::Block, scope);
            self.walk_children_excluding_fields(n, s, &[]);
            return;
        }
        match kind {
            "method_declaration" | "constructor_declaration" => {
                let s = self.open_scope(ScopeKind::Function, scope);
                self.walk_children_excluding_fields(n, s, &[]);
            }
            "local_variable_declaration" => {
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        self.bind_declarator(child, scope, true);
                    } else {
                        self.walk(child, scope);
                    }
                }
            }
            "assignment_expression" => {
                let left = n.child_by_field_name("left");
                let op_plain = n
                    .child_by_field_name("operator")
                    .and_then(|o| o.utf8_text(self.src).ok())
                    == Some("=");
                if let Some(left) = left {
                    if left.kind() == "identifier" {
                        let right = n.child_by_field_name("right");
                        let w = Write {
                            byte: left.start_byte(),
                            node_id: left.id(),
                            plain: true,
                            rhs: if op_plain {
                                right.map(|r| r.id())
                            } else {
                                None
                            },
                        };
                        let name = self.text(left).to_string();
                        let intro = if op_plain {
                            IntroKind::Assign
                        } else {
                            IntroKind::Binding
                        };
                        self.bind(scope, &name, w, intro);
                        if !op_plain {
                            self.record_read(scope, &name, left.end_byte());
                        }
                        // compound assignments read the previous value too
                    } else {
                        // field/array targets: operands are reads only
                        self.walk_children(left, scope);
                    }
                }
                if let Some(right) = n.child_by_field_name("right") {
                    self.walk(right, scope);
                }
            }
            "augmented_assignment_expression" => {
                if let Some(left) = n.child_by_field_name("left") {
                    if left.kind() == "identifier" {
                        let byte = left.start_byte();
                        let w = Write {
                            byte,
                            node_id: left.id(),
                            plain: false,
                            rhs: None,
                        };
                        let name = self.text(left).to_string();
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
            "enhanced_for_statement" => {
                // the control variable is loop protocol -- never tracked;
                // the iterated collection still produces its reads
                let s = self.open_scope(ScopeKind::Block, scope);
                self.walk_children_excluding_fields(n, s, &["name"]);
            }
            "for_statement" => {
                // the head declaration binds a protocol variable and is
                // not tracked; condition and updates walk normally
                let s = self.open_scope(ScopeKind::Block, scope);
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    match child.kind() {
                        "local_variable_declaration" => {
                            // bind head names so body reads resolve locally
                            let mut dc = child.walk();
                            for d in child.children(&mut dc) {
                                if d.kind() == "variable_declarator" {
                                    self.bind_declarator(d, s, false);
                                }
                            }
                        }
                        _ => self.walk(child, s),
                    }
                }
            }
            "catch_clause" => {
                let s = self.open_scope(ScopeKind::Block, scope);
                let binder = child_of_kind(n, "catch_formal_parameter");
                if let Some(bp) = binder {
                    if let Some(name_node) = bp.child_by_field_name("name") {
                        let w = Write {
                            byte: name_node.start_byte(),
                            node_id: name_node.id(),
                            plain: false,
                            rhs: None,
                        };
                        let name = self.text(name_node).to_string();
                        self.bind(s, &name, w, IntroKind::Binding);
                    }
                }
                let skipped = binder.map(|b| b.id());
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if Some(child.id()) == skipped {
                        continue;
                    }
                    self.walk(child, s);
                }
            }
            "resource" => {
                // try-with-resources binding
                if let Some(name_node) = n.child_by_field_name("name").filter(|n| n.kind() == "identifier") {
                    let w = Write {
                        byte: name_node.start_byte(),
                        node_id: name_node.id(),
                        plain: false,
                        rhs: None,
                    };
                    let name = self.text(name_node).to_string();
                    self.bind(scope, &name, w, IntroKind::Binding);
                }
                self.walk_children_excluding_fields(n, scope, &["name"]);
            }
            "method_invocation" => {
                // the @name slot is a member reference, not a variable
                self.walk_children_excluding_fields(n, scope, &["name"]);
            }
            "field_access" => {
                // the @field slot is a member reference, not a variable
                self.walk_children_excluding_fields(n, scope, &["field"]);
            }
            "identifier" => {
                let name = self.text(n).to_string();
                self.record_read(scope, &name, n.start_byte());
            }
            _ => self.walk_children_excluding_fields(n, scope, &[]),
        }
    }
}

fn child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).find(|ch| ch.kind() == kind)
}

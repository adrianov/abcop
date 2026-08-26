//! Variable write/read model powering UsedOnce/NeverUsed for Go.
//!
//! Scope rules mirror the Rust backend: reads resolve through Block
//! scopes -- which here include function literals, explicit blocks and
//! the implicit scopes of if/for/switch statements -- but stop at
//! Function boundaries. A read before the binding's introduction never
//! counts; Go rejects use-before-declaration at compile time anyway.

use std::collections::HashMap;

use tree_sitter::Node;

use super::GoFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// `:=`, `=` first introduction, var spec -- inline candidate
    Assign,
    /// compound assignment or inc/dec -- never a candidate
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

/// Statements/subtrees carrying no variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "parameter_list",
    "import_declaration",
    "import_spec",
    "import_spec_list",
];

const VETO_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "expression_switch_statement",
    "type_switch_statement",
    "select_statement",
];

pub fn used_once_offenses(fm: &GoFile) -> Vec<UsedOnceOffense> {
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

pub fn never_used_offenses(fm: &GoFile) -> Vec<NeverUsedOffense> {
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
    fm: &'t GoFile<'t>,
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

/// Conservative RHS purity: literals and operator compositions over them.
/// References to other locals, calls and composite literals are rejected,
/// mirroring the Rust backend.
fn pure(n: Node) -> bool {
    match n.kind() {
        "int_literal"
        | "float_literal"
        | "imaginary_literal"
        | "rune_literal"
        | "raw_string_literal"
        | "interpreted_string_literal"
        | "true"
        | "false"
        | "nil"
        | "iota" => true,
        "parenthesized_expression" => n.named_child(0).map(pure).unwrap_or(false),
        "binary_expression" => {
            let mut c = n.walk();
            n.children(&mut c).all(pure)
        }
        "unary_expression" => {
            let mut c = n.walk();
            let mut kids = n.children(&mut c);
            let first = kids.next();
            let _ = first;
            kids.all(pure) && unary_op_ok(n)
        }
        _ => false,
    }
}

/// Unary operators that keep an expression constant-foldable; `&` and
/// `<-` create references / channel receives and are rejected.
fn unary_op_ok(n: Node) -> bool {
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        if !ch.is_named() {
            return matches!(
                ch.utf8_text(b"").unwrap_or(""),
                "-" | "+" | "^" | "!"
            );
        }
    }
    false
}

/// Straight-line execution check up to the nearest Function boundary;
/// bare blocks do not break straight-line execution.
fn unconditionally_executed(write_node: Node) -> bool {
    const OWNERS: [&str; 2] = ["function_declaration", "method_declaration"];
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

/// Statement kinds that open an implicit Block scope for their clause
/// variables and bodies.
const BLOCK_SCOPED: &[&str] = &[
    "block",
    "function_literal",
    "if_statement",
    "for_statement",
    "expression_switch_statement",
    "type_switch_statement",
    "select_statement",
];

impl Collector<'_> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
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

    fn identifier_targets<'t>(left: Node<'t>, out: &mut Vec<Node<'t>>) {
        if left.kind() == "identifier" {
            out.push(left);
            return;
        }
        let mut cursor = left.walk();
        for child in left.children(&mut cursor) {
            Self::identifier_targets(child, out);
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
            "function_declaration" | "method_declaration" => {
                let s = self.open_scope(ScopeKind::Function, scope);
                self.walk_children(n, s);
            }
            "short_var_declaration" => {
                self.bind_assignment(n, scope);
                if let Some(right) = n.child_by_field_name("right") {
                    self.walk(right, scope);
                }
            }
            "assignment_statement" => {
                // plain `=` vs compound: distinguishable by operator token
                let op_text = self.first_anon_op(n);
                if op_text == Some("=") {
                    self.bind_assignment(n, scope);
                } else {
                    self.bind_compound(n, scope);
                }
                if let Some(right) = n.child_by_field_name("right") {
                    self.walk(right, scope);
                }
            }
            "var_spec" => {
                self.bind_var_spec(n, scope);
            }
            "inc_statement" | "dec_statement" => {
                // i++ / i-- : reads and rewrites
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    if child.kind() == "identifier" {
                        let byte = child.start_byte();
                        let w = Write {
                            byte,
                            node_id: child.id(),
                            plain: false,
                            rhs: None,
                        };
                        let name = self.text(child).to_string();
                        self.bind(scope, &name, w, IntroKind::Binding);
                        self.record_read(scope, &name, byte + 1);
                    } else {
                        self.walk(child, scope);
                    }
                }
            }
            "range_clause" => {
                // range variables are loop protocol, never tracked
                let skipped = n.child_by_field_name("left").map(|l| l.id());
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if Some(child.id()) != skipped {
                        self.walk(child, scope);
                    }
                }
            }
            "identifier" => {
                let name = self.text(n).to_string();
                self.record_read(scope, &name, n.start_byte());
            }
            _ => self.walk_children(n, scope),
        }
    }

    fn first_anon_op<'s>(&'s self, n: Node<'s>) -> Option<&'s str> {
        let mut c = n.walk();
        n.children(&mut c)
            .find(|ch| !ch.is_named())
            .map(|ch| ch.utf8_text(self.src).unwrap_or(""))
    }

    /// Plain `=` / `:=`: each expression-list element either binds an
    /// identifier or is a reference whose operands become reads.
    fn bind_assignment(&mut self, n: Node, scope: usize) {
        let Some(left) = n.child_by_field_name("left") else {
            return;
        };
        let right = n.child_by_field_name("right");
        let mut targets = Vec::new();
        Self::identifier_targets(left, &mut targets);
        let single = targets.len() == 1;
        for t in targets {
            let rhs = if single {
                right.and_then(|r| r.named_child(0)).map(|v| v.id())
            } else {
                None
            };
            let w = Write {
                byte: t.start_byte(),
                node_id: t.id(),
                plain: true,
                rhs,
            };
            let name = self.text(t).to_string();
            self.bind(scope, &name, w, IntroKind::Assign);
        }
        // reference-style elements (t.n = ..., m[k] = ...): operand reads
        let mut cursor = left.walk();
        for element in left.children(&mut cursor) {
            if element.kind() != "identifier" {
                self.walk(element, scope);
            }
        }
    }

    fn bind_compound(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        if let Some(left) = left {
            let mut targets = Vec::new();
            Self::identifier_targets(left, &mut targets);
            for t in targets {
                let byte = t.start_byte();
                let w = Write {
                    byte,
                    node_id: t.id(),
                    plain: false,
                    rhs: None,
                };
                let name = self.text(t).to_string();
                self.bind(scope, &name, w, IntroKind::Binding);
                self.record_read(scope, &name, byte + 1);
            }
        }
    }

    /// `var u, w = v, *p`: declared names precede the `=` token; values
    /// follow. RHS links only when one name maps to one value.
    fn bind_var_spec(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        let mut names = Vec::new();
        let mut values = Vec::new();
        let mut past_eq = false;
        for child in children {
            if !child.is_named() && self.text(child) == "=" {
                past_eq = true;
                continue;
            }
            if child.is_named() && child.kind() == "identifier" && !past_eq {
                names.push(child);
            } else if past_eq {
                values.push(child);
            }
        }
        let single_pair = names.len() == 1 && values.len() == 1;
        for (idx, t) in names.iter().enumerate() {
            let rhs = if single_pair {
                values[0].id().into()
            } else {
                values.get(idx).map(|v| v.id())
            };
            let w = Write {
                byte: t.start_byte(),
                node_id: t.id(),
                plain: true,
                rhs,
            };
            let name = self.text(*t).to_string();
            self.bind(scope, &name, w, IntroKind::Assign);
        }
        for v in values {
            self.walk(v, scope);
        }
    }
}

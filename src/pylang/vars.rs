//! Variable write/read model powering UsedOnce/NeverUsed for Python.
//!
//! Scope rules mirror the Rust backend: reads resolve through Block
//! scopes (lambda bodies roll up like Rust closures) but stop at
//! Function/Class boundaries (nested defs are independent scopes). Reads
//! before the binding's introduction position never count -- Python
//! raises UnboundLocalError for exactly that pattern.

use std::collections::HashMap;

use tree_sitter::Node;

use super::PyFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// plain assignment / walrus -- inline candidate
    Assign,
    /// augmented assignment or pattern capture -- never a candidate
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
    Class,
    Block,
}

pub(super) struct Scope {
    parent: Option<usize>,
    kind: ScopeKind,
    entries: HashMap<Box<str>, Entry>,
}

/// Statements whose subtrees carry no variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "parameters",
    "import_statement",
    "import_from_statement",
    "future_import_statement",
    "global_statement",
    "nonlocal_statement",
];

/// Control-flow ancestors that disqualify a write from inlining.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "elif_clause",
    "else_clause",
    "for_statement",
    "while_statement",
    "try_statement",
    "except_clause",
    "finally_clause",
    "match_statement",
    "case_clause",
    "with_statement",
    "conditional_expression",
];

pub(crate) fn used_once_offenses(fm: &PyFile) -> Vec<UsedOnceOffense> {
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

pub(crate) fn never_used_offenses(fm: &PyFile) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if !e.reads.is_empty() || e.writes.is_empty() {
                continue;
            }
            let (line, column) = fm.line_col(e.writes[0].byte);
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
    fm: &'t PyFile<'t>,
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
/// References to other locals are rejected, mirroring the Rust backend.
fn pure(n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "true" | "false" | "none" => true,
        "string" => children_pure(n),
        "string_content" | "escape_sequence" => true,
        "list" | "tuple" | "set" | "dictionary" | "pair" | "unary_operator"
        | "binary_operator" | "boolean_operator" | "parenthesized_expression" => {
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

/// Straight-line execution check up to the nearest scope boundary.
fn unconditionally_executed(write_node: Node) -> bool {
    const OWNERS: [&str; 3] = ["function_definition", "class_definition", "lambda"];
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

/// Build the scope tree and per-identifier bookkeeping for a parsed file.
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
    fn open_scope(&mut self, kind: ScopeKind, parent: usize) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    /// Name exists in this scope (Python locals are scope-wide, so a
    /// same-scope name always shadows outer scopes regardless of
    /// position); the read only counts when the binding was already
    /// introduced at the read position.
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

    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    /// Walk every child except the given field's subtree.
    fn walk_except(&mut self, n: Node, scope: usize, skip_field: &str) {
        let skipped = n.child_by_field_name(skip_field).map(|s| s.id());
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if Some(child.id()) == skipped {
                continue;
            }
            self.walk(child, scope);
        }
    }

    /// Bind every name *written* by an assignment target expression.
    /// Pattern lists expand per bound name; attribute (`obj.attr =`) and
    /// subscript (`obj[k] =`) targets merely reference their operands --
    /// those operands become ordinary reads instead.
    fn bind_targets(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "identifier" => {
                let w = Write {
                    byte: n.start_byte(),
                    node_id: n.id(),
                    plain: true,
                    rhs: None,
                };
                let name = self.text(n).to_string();
                self.bind(scope, &name, w, IntroKind::Assign);
            }
            // expand pattern lists per bound name; must NOT recurse
            // through walk(), which would treat names as reads
            "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat"
            | "dictionary_splat" => {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    self.bind_targets(child, scope);
                }
            }
            // reference-style targets (obj.attr / obj[k]): operands are reads
            _ => self.walk_children(n, scope),
        }
    }

    /// Track an `as <name>` protocol binding (with/except/match aliases)
    /// as a non-candidate write.
    fn bind_alias(&mut self, n: Node, scope: usize) {
        if n.kind() == "identifier" {
            let w = Write {
                byte: n.start_byte(),
                node_id: n.id(),
                plain: false,
                rhs: None,
            };
            let name = self.text(n).to_string();
            self.bind(scope, &name, w, IntroKind::Binding);
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.bind_alias(child, scope);
        }
    }

    /// Bind every identifier inside a match `case` pattern as a capture
    /// (never an inline candidate).
    fn bind_captures(&mut self, n: Node, scope: usize) {
        if n.kind() == "identifier" {
            let w = Write {
                byte: n.start_byte(),
                node_id: n.id(),
                plain: false,
                rhs: None,
            };
            let name = self.text(n).to_string();
            self.bind(scope, &name, w, IntroKind::Binding);
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.bind_captures(child, scope);
        }
    }

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        match kind {
            "function_definition" => {
                let s = self.open_scope(ScopeKind::Function, scope);
                self.walk_except(n, s, "name");
            }
            "class_definition" => {
                let s = self.open_scope(ScopeKind::Class, scope);
                self.walk_except(n, s, "name");
            }
            "lambda" => {
                // Lambda bodies roll up like Rust closures: reads inside
                // still resolve outward through the Block scope.
                let s = self.open_scope(ScopeKind::Block, scope);
                self.walk_except(n, s, "parameters");
            }
            "assignment" => {
                let left = n.child_by_field_name("left");
                let right = n.child_by_field_name("right");
                if let Some(left) = left {
                    if left.kind() == "identifier" {
                        let w = Write {
                            byte: left.start_byte(),
                            node_id: left.id(),
                            plain: true,
                            rhs: right.map(|r| r.id()),
                        };
                        let name = self.text(left).to_string();
                        self.bind(scope, &name, w, IntroKind::Assign);
                    } else {
                        self.bind_targets(left, scope);
                    }
                }
                if let Some(right) = right {
                    self.walk(right, scope);
                }
            }
            "augmented_assignment" => {
                // reads the previous value and rewrites: neither candidate
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
                        // obj.x += 1 / d[k] += 1: operands are reads only
                        self.walk_children(left, scope);
                    }
                }
                if let Some(right) = n.child_by_field_name("right") {
                    self.walk(right, scope);
                }
            }
            "named_expression" => {
                if let (Some(name_node), Some(value)) = (
                    n.child_by_field_name("name"),
                    n.child_by_field_name("value"),
                ) {
                    let w = Write {
                        byte: name_node.start_byte(),
                        node_id: name_node.id(),
                        plain: true,
                        rhs: Some(value.id()),
                    };
                    let name = self.text(name_node).to_string();
                    self.bind(scope, &name, w, IntroKind::Assign);
                    self.walk(value, scope);
                }
            }
            "for_statement" | "for_in_clause" => {
                // loop targets are protocol bindings, never tracked
                self.walk_except(n, scope, "left");
            }
            "keyword_argument" => {
                // the label is not a variable reference
                if let Some(value) = n.child_by_field_name("value") {
                    self.walk(value, scope);
                }
            }
            "attribute" => {
                // the member name after the dot is not a variable read
                if let Some(obj) = n.child_by_field_name("object") {
                    self.walk(obj, scope);
                }
            }
            "as_pattern" => {
                // value part walks normally; everything after the `as`
                // token is the alias binding
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                let mut after_as = false;
                for child in children {
                    if child.kind() == "as" {
                        after_as = true;
                    } else if after_as {
                        self.bind_alias(child, scope);
                    } else {
                        self.walk(child, scope);
                    }
                }
            }
            "case_clause" => {
                // pattern captures bind names; guard and body are code
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    if child.kind() == "case_pattern" {
                        self.bind_captures(child, scope);
                    } else {
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
}

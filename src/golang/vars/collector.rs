//! Scope-model collection: builds the `Scope` tree from the syntax tree
//! and records every variable write/read into it.

use std::collections::HashMap;

use tree_sitter::Node;

use super::{Entry, IntroKind, Scope, ScopeKind, Write};

/// Statements/subtrees carrying no variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "parameter_list",
    "import_declaration",
    "import_spec",
    "import_spec_list",
];

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

pub(crate) fn collect(root: Node, src: &[u8]) -> Vec<Scope> {
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

pub(super) struct Collector<'a> {
    src: &'a [u8],
    scopes: Vec<Scope>,
}

impl Collector<'_> {
    pub(super) fn text(&self, n: Node) -> &str {
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

    /// Resolve a name to the scope id holding its entry, honoring Block
    /// fall-through and the read-must-follow-write rule.
    fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let data = &self.scopes[scope];
        if let Some(e) = data.entries.get(name) {
            return if e.intro_byte <= pos {
                Some(scope)
            } else {
                None
            };
        }
        match data.kind {
            ScopeKind::Block => self.lookup(data.parent?, pos, name),
            _ => None,
        }
    }

    pub(super) fn bind(&mut self, scope: usize, name: &str, w: Write, intro: IntroKind) {
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

    pub(super) fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
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

    pub(super) fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        if BLOCK_SCOPED.contains(&kind) {
            let s = self.open_scope(ScopeKind::Block, scope);
            self.walk_children(n, s);
            return;
        }
        if !self.bind_or_read(n, scope) {
            self.default_walk(n, kind, scope);
        }
    }

    /// Function declarations open Function scopes; anything unrecognized
    /// recurses through the current scope.
    fn default_walk(&mut self, n: Node, kind: &str, scope: usize) {
        if matches!(kind, "function_declaration" | "method_declaration") {
            let s = self.open_scope(ScopeKind::Function, scope);
            self.walk_children(n, s);
            return;
        }
        self.walk_children(n, scope);
    }

    pub(super) fn first_anon_op<'s>(&'s self, n: Node<'s>) -> Option<&'s str> {
        let mut c = n.walk();
        n.children(&mut c)
            .find(|ch| !ch.is_named())
            .map(|ch| ch.utf8_text(self.src).unwrap_or(""))
    }
}

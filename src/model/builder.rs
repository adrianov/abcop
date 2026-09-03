//! The model builder core: walks the tree-sitter tree, dispatching each node
//! to the scope / write / read handler groups, and records what it sees.

use std::collections::HashMap;

use tree_sitter::Node;

use super::{Entry, ScopeData, ScopeId, ScopeKind, Write};

pub(super) struct Builder<'m> {
    pub(super) src: &'m [u8],
    pub(super) scopes: &'m mut Vec<ScopeData>,
    pub(super) csend_sites: &'m mut Vec<(usize, Box<str>, ScopeId)>,
    pub(super) vcall_sites: &'m mut Vec<usize>,
}

impl<'m> Builder<'m> {
    pub(super) fn text<'t>(&'t self, n: Node<'t>) -> &'t str {
        n.utf8_text(self.src).unwrap_or("")
    }

    pub(super) fn scope_for(
        &mut self,
        owner: Node,
        kind: ScopeKind,
        parent: Option<ScopeId>,
    ) -> ScopeId {
        self.scopes.push(ScopeData {
            parent,
            kind,
            entered_at: owner.start_byte(),
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    pub(super) fn lookup(&self, scope: ScopeId, pos: usize, name: &str) -> Option<ScopeId> {
        super::lookup_scope(self.scopes, scope, pos, name)
    }

    /// Record a write; creates the binding if not visible yet.
    pub(super) fn record_write(
        &mut self,
        scope: ScopeId,
        name: &str,
        w: Write,
        intro_kind: super::IntroKind,
    ) {
        if name.starts_with('_') {
            return;
        }
        let pos = w.byte;
        match self.lookup(scope, pos, name) {
            Some(s) => {
                self.scopes[s].entries.get_mut(name).unwrap().writes.push(w);
            }
            None => {
                let e = Entry {
                    intro_byte: pos,
                    intro_kind,
                    writes: vec![w],
                    reads: Vec::new(),
                };
                self.scopes[scope].entries.insert(name.into(), e);
            }
        }
    }

    pub(super) fn record_read(&mut self, scope: ScopeId, name: &str, r: super::Read) {
        let Some(s) = self.lookup(scope, r.byte, name) else {
            return;
        };

        self.scopes[s].entries.get_mut(name).unwrap().reads.push(r);
    }

    /// Bind a parameter-style entry only when no binding exists yet.
    pub(super) fn bind_entry(&mut self, scope: ScopeId, name: String, pos: usize) {
        self.scopes[scope]
            .entries
            .entry(name.into())
            .or_insert(Entry {
                intro_byte: pos,
                intro_kind: super::IntroKind::Binding,
                writes: Vec::new(),
                reads: Vec::new(),
            });
    }

    pub(super) fn walk(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        let kind = n.kind();
        if self.walk_scope_intro(n, kind, scope) {
            return;
        }
        if self.walk_write(n, kind, scope, under_defined) {
            return;
        }
        if self.walk_read(n, kind, scope, under_defined) {
            return;
        }
        self.walk_children(n, scope, under_defined);
    }

    /// Generic descent.
    fn walk_children(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        for child in n.children(&mut n.walk()) {
            if child.kind() == "method_parameters"
                || child.kind() == "block_parameters"
                || child.kind() == "lambda_parameters"
            {
                // stray parameter container outside a callable; descend into
                // default-value expressions only
                let mut sub = child.walk();
                for inner in child.children(&mut sub) {
                    if inner.child_by_field_name("value").is_some() {
                        self.walk(inner, scope, under_defined);
                    }
                }
                continue;
            }
            self.walk(child, scope, under_defined);
        }
    }
}

pub(super) fn declared_name(child: Node, src: &[u8]) -> Option<String> {
    match child.kind() {
        "identifier" => Some(child.utf8_text(src).unwrap_or("").to_string()),
        "optional_parameter" | "keyword_parameter" | "block_parameter" | "splat_parameter" => child
            .child_by_field_name("name")
            .or_else(|| {
                child
                    .children(&mut child.walk())
                    .find(|c| c.kind() == "identifier")
            })
            .map(|n| n.utf8_text(src).unwrap_or("").to_string()),
        _ => None,
    }
}

/// Body of a block-ish node regardless of brace/do form.
pub(super) fn body_of(n: Node) -> Option<Node> {
    n.children(&mut n.walk())
        .find(|c| c.kind() == "body_statement" || c.kind() == "block_body")
}

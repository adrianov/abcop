//! Shared variable-scope model behind UsedOnce/NeverUsed.
//!
//! This module splits in two:
//! * [`mod@self`] (here) -- the scope-tree data structure: kinds, the
//!   write/read [`Entry`] bookkeeping and the introduction-aware
//!   [`Model`] collectors drive while walking;
//! * [`eval`] -- candidate evaluation over that tree, parameterized by
//!   a per-language [`Semantics`].
//!
//! Scope resolution contract: a name resolves to the nearest enclosing
//! scope that already introduced it at the read position; resolution
//! escapes through [`ScopeKind::Block`] scopes (closures, nested
//! blocks) and stops at any other kind (functions, methods).

mod backend;
mod eval;
pub mod walk;

use std::collections::HashMap;

use tree_sitter::Node;

pub use eval::{never_used_offenses, used_once_offenses, Semantics};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntroKind {
    /// first introduction is an ordinary binding -- inline candidate
    Assign,
    /// introduced by a rewrite (compound assignment, catch/resource
    /// binding) -- never a candidate
    Binding,
}

#[derive(Clone, Copy, Debug)]
pub struct Write {
    pub byte: usize,
    pub node_id: usize,
    pub plain: bool,
    pub rhs: Option<usize>,
}

impl Write {
    /// A rebindable write whose value can be linked to an expression.
    pub fn assign(byte: usize, node_id: usize, rhs: Option<usize>) -> Write {
        Write {
            byte,
            node_id,
            plain: true,
            rhs,
        }
    }

    /// A rewriting touch (`+=`, catch/resource head): reads the previous
    /// value and can never be inlined away.
    pub fn rewrite(byte: usize, node_id: usize) -> Write {
        Write {
            byte,
            node_id,
            plain: false,
            rhs: None,
        }
    }
}

#[derive(Debug)]
pub struct Entry {
    pub intro_byte: usize,
    pub intro_kind: IntroKind,
    pub writes: Vec<Write>,
    pub reads: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Root,
    Function,
    Block,
}

#[derive(Debug)]
pub struct Scope {
    pub parent: Option<usize>,
    pub kind: ScopeKind,
    pub entries: HashMap<Box<str>, Entry>,
}

/// Bookkeeping over a scope tree: introduction-aware lookups and
/// write/read recording. Collectors drive it while walking.
#[derive(Debug, Default)]
pub struct Model {
    pub scopes: Vec<Scope>,
}

impl Model {
    pub fn rooted() -> Model {
        Model {
            scopes: vec![Scope {
                parent: None,
                kind: ScopeKind::Root,
                entries: HashMap::new(),
            }],
        }
    }

    pub fn open_scope(&mut self, kind: ScopeKind, parent: usize) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    /// Name exists in this scope (a same-scope declaration shadows outer
    /// scopes regardless of position); the use counts only when the
    /// binding was already introduced at `pos`.
    pub fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let data = &self.scopes[scope];
        if let Some(e) = data.entries.get(name) {
            return if e.intro_byte <= pos { Some(scope) } else { None };
        }
        match data.kind {
            ScopeKind::Block => self.lookup(data.parent?, pos, name),
            _ => None,
        }
    }

    pub fn bind(&mut self, scope: usize, name: &str, w: Write, intro: IntroKind) {
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

    pub fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
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
}

/// First child of the given kind -- the small lookup every collector
/// ends up needing for protocol heads and bindings.
pub(crate) fn child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).find(|ch| ch.kind() == kind)
}

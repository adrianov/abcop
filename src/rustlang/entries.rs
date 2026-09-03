//! Scope-entry bookkeeping: opening scopes, resolving names, recording
//! writes and reads.

use std::collections::HashMap;

use super::builder::Builder;
use super::scope::{Entry, IntroKind, Scope, ScopeKind, Write};

impl<'m> Builder<'m> {
    pub(super) fn open_scope(&mut self, kind: ScopeKind, parent: Option<usize>) -> usize {
        self.scopes.push(Scope {
            parent,
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    /// Nearest enclosing scope (function scopes are opaque) where `name` is
    /// already introduced at `pos`.
    pub(super) fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let data = &self.scopes[scope];
        if self.introduced(data, pos, name) {
            return Some(scope);
        }
        // Rust scoping is purely lexical; function scopes are opaque
        match data.kind {
            ScopeKind::Block => self.lookup(data.parent?, pos, name),
            _ => None,
        }
    }

    fn introduced(&self, scope: &Scope, pos: usize, name: &str) -> bool {
        scope.entries.get(name).is_some_and(|e| e.intro_byte <= pos)
    }
    pub(super) fn record_write(&mut self, scope: usize, name: &str, w: Write, intro: IntroKind) {
        if name.starts_with('_') {
            return;
        }
        match self.lookup(scope, w.byte, name) {
            Some(s) => {
                self.scopes[s].entries.get_mut(name).unwrap().writes.push(w);
            }
            None => {
                let e = Entry {
                    intro_byte: w.byte,
                    intro_kind: intro,
                    writes: vec![w],
                    reads: Vec::new(),
                    macro_reads: 0,
                };
                self.scopes[scope].entries.insert(name.into(), e);
            }
        }
    }

    pub(super) fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
        let Some(s) = self.lookup(scope, byte, name) else {
            return;
        };
        let in_macro = self.macro_depth > 0;
        let e = self.scopes[s].entries.get_mut(name).unwrap();
        e.reads.push(byte);
        if in_macro {
            e.macro_reads += 1;
        }
    }
}

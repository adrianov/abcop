//! Binder node handling: lets, assignments, match arms, loops, params.

use std::collections::HashSet;

use tree_sitter::Node;

use super::builder::Builder;
use super::patterns::{match_binders, pattern_identifiers};
use super::scope::{Entry, IntroKind, Write};

impl<'m> Builder<'m> {
    pub(super) fn bind_pattern(&mut self, pattern: Option<Node>, scope: usize, intro: IntroKind) {
        let Some(p) = pattern else { return };
        let mut ids = Vec::new();
        pattern_identifiers(p, self.src, &mut ids);
        for id in ids {
            let name = self.text(id).to_string();
            self.record_write(
                scope,
                &name,
                Write {
                    byte: id.start_byte(),
                    node_id: id.id(),
                    plain: intro == IntroKind::Assign,
                    rhs: None,
                },
                intro,
            );
        }
    }

    pub(super) fn handle_let(&mut self, n: Node, scope: usize) {
        let pattern = n.child_by_field_name("pattern");
        let value = n.child_by_field_name("value");
        if let Some(p) = pattern {
            self.bind_let_ids(p, value, scope);
        }
        if let Some(v) = value {
            self.walk(v, scope);
        }
    }

    fn bind_let_ids(&mut self, p: Node, value: Option<Node>, scope: usize) {
        let mut ids = Vec::new();
        pattern_identifiers(p, self.src, &mut ids);
        for id in ids {
            let name = self.text(id).to_string();
            self.record_write(
                scope,
                &name,
                Write {
                    byte: id.start_byte(),
                    node_id: id.id(),
                    plain: true,
                    rhs: value.map(|v| (v.id(), v.start_byte())),
                },
                IntroKind::Assign,
            );
        }
    }

    pub(super) fn handle_assign(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        if let Some(l) = left {
            self.assign_left(l, scope);
        }
        if let Some(r) = right {
            self.walk(r, scope);
        }
    }

    /// A bare identifier on the left is a write; anything else (field,
    /// index, …) is walked as reads.
    fn assign_left(&mut self, l: Node, scope: usize) {
        if l.kind() != "identifier" {
            self.walk(l, scope);
            return;
        }
        let name = self.text(l).to_string();
        self.record_write(
            scope,
            &name,
            Write {
                byte: l.start_byte(),
                node_id: l.id(),
                plain: false,
                rhs: None,
            },
            IntroKind::Binding,
        );
    }

    pub(super) fn handle_match_arm(&mut self, n: Node, scope: usize) {
        if let Some(p) = n.child_by_field_name("pattern") {
            let skip = self.bind_match_arm(p, scope);
            self.walk_skip_ids(p, scope, &skip);
        }
        if let Some(v) = n.child_by_field_name("value") {
            self.walk(v, scope);
        }
    }

    /// Record arm binders as writes; return their ids to skip on the walk.
    fn bind_match_arm(&mut self, p: Node, scope: usize) -> HashSet<usize> {
        let mut binders = Vec::new();
        match_binders(p, self.src, &mut binders);
        for id in &binders {
            let name = self.text(*id).to_string();
            self.record_write(
                scope,
                &name,
                Write {
                    byte: id.start_byte(),
                    node_id: id.id(),
                    plain: false,
                    rhs: None,
                },
                IntroKind::Binding,
            );
        }
        binders.iter().map(|b| b.id()).collect()
    }

    pub(super) fn handle_loop_or_let_binding(&mut self, n: Node, scope: usize) {
        self.bind_pattern(n.child_by_field_name("pattern"), scope, IntroKind::Binding);
        let pat = n.child_by_field_name("pattern");
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if pat.map(|p| p.id()) == Some(child.id()) {
                continue;
            }
            self.walk(child, scope);
        }
    }

    pub(super) fn declare_params(&mut self, container: Node, scope: usize) {
        let mut cursor = container.walk();
        for child in container.children(&mut cursor) {
            match child.kind() {
                "parameter" => self.declare_parameter(child, scope),
                "identifier" => self.declare_ident(child, scope),
                _ => {}
            }
        }
    }

    /// A typed `parameter` node binds through its inner pattern.
    fn declare_parameter(&mut self, param: Node, scope: usize) {
        let Some(pat) = param.child_by_field_name("pattern") else {
            return;
        };
        let mut ids = Vec::new();
        pattern_identifiers(pat, self.src, &mut ids);
        for id in ids {
            let name = self.text(id).to_string();
            if !name.starts_with('_') {
                self.introduce_binding(name, id.start_byte(), scope);
            }
        }
    }

    /// Closure parameters may be bare identifiers.
    fn declare_ident(&mut self, ident: Node, scope: usize) {
        let name = self.text(ident).to_string();
        if !name.starts_with('_') {
            self.introduce_binding(name, ident.start_byte(), scope);
        }
    }

    fn introduce_binding(&mut self, name: String, pos: usize, scope: usize) {
        self.scopes[scope]
            .entries
            .entry(name.into())
            .or_insert(Entry {
                intro_byte: pos,
                intro_kind: IntroKind::Binding,
                writes: Vec::new(),
                reads: Vec::new(),
                macro_reads: 0,
            });
    }
}

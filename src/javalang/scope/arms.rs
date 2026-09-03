//! Kind-dispatch arms of the Java collector walk: every syntactic shape
//! gets its own helper so [`super::Collector`] traversal stays free of
//! node-kind logic.

use tree_sitter::Node;

use super::{Collector, EXCLUDE_FIELDS, SKIP_KINDS};
use crate::scope_model::{IntroKind, ScopeKind, Write, child_of_kind};

impl Collector<'_> {
    /// Consume nodes handled before the kind dispatch: whole subtrees
    /// with no local activity (`SKIP_KINDS`), and expressions whose
    /// member slot would corrupt the read path. Returns true when the
    /// node was fully handled.
    pub(super) fn shortcut(&mut self, n: Node, scope: usize, kind: &str) -> bool {
        if SKIP_KINDS.contains(&kind) {
            return true;
        }
        if let Some((_, field)) = EXCLUDE_FIELDS.iter().find(|(k, _)| *k == kind)
            && let Some(excluded) = n.child_by_field_name(field).map(|c| c.id())
        {
            n.children(&mut n.walk())
                .filter(|c| c.id() != excluded)
                .for_each(|c| self.walk(c, scope));
            return true;
        }
        false
    }

    /// Everything reaching the dispatch opens scopes, binds or reads.
    pub(super) fn arm(&mut self, n: Node, scope: usize, kind: &str) {
        match kind {
            "method_declaration" | "constructor_declaration" => {
                let s = self.model.open_scope(ScopeKind::Function, scope);
                self.walk_children(n, s);
            }
            "local_variable_declaration" => self.declaration_arm(n, scope),
            "assignment_expression" | "augmented_assignment_expression" => {
                self.assignment_arm(n, scope);
            }
            "for_statement" => self.for_arm(n, scope),
            "catch_clause" => self.catch_arm(n, scope),
            "resource" => self.resource_arm(n, scope),
            "identifier" => {
                self.model
                    .record_read(scope, &self.text(n).to_string(), n.start_byte());
            }
            _ => self.walk_children(n, scope),
        }
    }

    fn declaration_arm(&mut self, n: Node, scope: usize) {
        for child in n.children(&mut n.walk()) {
            match child.kind() {
                "variable_declarator" => self.bind_declarator(child, scope, true),
                _ => self.walk(child, scope),
            }
        }
    }
    /// Head declarations bind protocol variables so body reads resolve
    /// locally; condition and updates walk normally.
    fn for_arm(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "local_variable_declaration" {
                child
                    .children(&mut child.walk())
                    .filter(|d| d.kind() == "variable_declarator")
                    .for_each(|d| self.bind_declarator(d, s, false));
            } else {
                self.walk(child, s);
            }
        }
    }

    fn catch_arm(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        // Bind the parameter's name but exclude the whole parameter
        // subtree: its variable_name must never leak into the read path.
        let bp = child_of_kind(n, "catch_formal_parameter");
        self.bind_catch_param(bp, s);

        n.children(&mut n.walk())
            .filter(|c| Some(c.id()) != bp.map(|b| b.id()))
            .for_each(|c| self.walk(c, s));
    }

    fn bind_catch_param(&mut self, bp: Option<Node>, scope: usize) {
        if let Some(bp) = bp
            && let Some(name) = bp.child_by_field_name("name")
        {
            self.bind_var(
                name,
                scope,
                Write::rewrite(name.start_byte(), name.id()),
                IntroKind::Binding,
            );
        }
    }

    /// try-with-resources binding; its initializer still reads.
    fn resource_arm(&mut self, n: Node, scope: usize) {
        if let Some(name) = n
            .child_by_field_name("name")
            .filter(|n| n.kind() == "identifier")
        {
            self.bind_var(
                name,
                scope,
                Write::rewrite(name.start_byte(), name.id()),
                IntroKind::Binding,
            );
        }
        if let Some(value) = n.child_by_field_name("value") {
            self.walk(value, scope);
        }
    }

    /// Assignments rebind variables -- plain `=` records one candidate
    /// write per identifier target, compound operators rewrite-and-read;
    /// field/array targets contribute operand reads only.
    fn assignment_arm(&mut self, n: Node, scope: usize) {
        let plain = self.op_text(n) == "=";
        if let Some(left) = n.child_by_field_name("left") {
            if left.kind() == "identifier" {
                self.bind_target(left, n, plain, scope);
            } else {
                self.walk_children(left, scope);
            }
        }
        // the left side was handled above; walk the remaining children
        self.walk_children(n, scope);
    }

    fn bind_target(&mut self, left: Node, n: Node, plain: bool, scope: usize) {
        let name = self.text(left).to_string();
        let (w, intro) = self.target_write(n, left, plain);
        self.model.bind(scope, &name, w, intro);
        if !plain {
            self.model.record_read(scope, &name, left.end_byte());
        }
    }

    /// Plain `=` links the RHS id as the inlinable write; compound
    /// operators rewrite in place.
    fn target_write(&self, n: Node, left: Node, plain: bool) -> (Write, IntroKind) {
        if plain {
            (
                Write::assign(
                    left.start_byte(),
                    left.id(),
                    n.child_by_field_name("right").map(|r| r.id()),
                ),
                IntroKind::Assign,
            )
        } else {
            (
                Write::rewrite(left.start_byte(), left.id()),
                IntroKind::Binding,
            )
        }
    }

    /// The node's textual operator, or "" when there is none.
    fn op_text(&self, n: Node) -> &str {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
            .unwrap_or("")
    }
}

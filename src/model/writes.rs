//! Write-recording constructs: assignments, operator-assignments, for and
//! rescue binders, and multiple-assignment target lists.

use tree_sitter::Node;

use super::builder::Builder;
use super::{IntroKind, ScopeId, Write, WriteKind};

impl Builder<'_> {
    /// Returns true when `kind` is a write-introducing construct.
    pub(super) fn walk_write(
        &mut self,
        n: Node,
        kind: &str,
        scope: ScopeId,
        under_defined: bool,
    ) -> bool {
        match kind {
            "assignment" => {
                self.walk_assignment(n, scope, under_defined);
                true
            }
            "operator_assignment" => {
                self.walk_operator_assignment(n, scope, under_defined);
                true
            }
            "for" => {
                self.walk_for(n, scope, under_defined);
                true
            }
            "rescue" => {
                self.walk_rescue(n, scope, under_defined);
                true
            }
            "when" | "in_clause" => {
                // pattern subtree may bind variables we deliberately do not
                // track; walk only the body
                if let Some(b) = n.child_by_field_name("body") {
                    self.walk(b, scope, under_defined);
                }
                true
            }
            _ => false,
        }
    }

    fn walk_assignment(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        let left = n.child_by_field_name("left");
        let rhs = n.child_by_field_name("right");
        match left {
            Some(l) if l.kind() == "left_assignment_list" => {
                self.collect_masgn_targets(l, scope);
                if let Some(r) = rhs {
                    self.walk(r, scope, under_defined);
                }
            }
            Some(l) if l.kind() == "identifier" => {
                self.record_identifier_write(l, rhs, scope, under_defined);
            }
            // attribute / element / ivar / const targets: not locals,
            // but the RHS may still contain reads
            _ => {
                if let Some(l) = left {
                    self.walk(l, scope, under_defined);
                }
                if let Some(r) = rhs {
                    self.walk(r, scope, under_defined);
                }
            }
        }
    }

    /// Plain `name = rhs`: record the write, then scan the RHS for reads.
    fn record_identifier_write(
        &mut self,
        l: Node,
        rhs: Option<Node>,
        scope: ScopeId,
        under_defined: bool,
    ) {
        let name = self.text(l).to_string();
        self.record_write(
            scope,
            &name,
            Write::at(l, WriteKind::Plain, rhs.map(|r| (r.id(), r.end_byte()))),
            IntroKind::Assign,
        );
        if let Some(r) = rhs {
            self.walk(r, scope, under_defined);
        }
    }

    fn walk_operator_assignment(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        if let Some(l) = n.child_by_field_name("left") {
            if l.kind() == "identifier" {
                let name = self.text(l).to_string();
                self.record_write(
                    scope,
                    &name,
                    Write::at(l, WriteKind::OpAssign, None),
                    IntroKind::Binding,
                );
            } else {
                self.walk(l, scope, under_defined);
            }
        }
        if let Some(r) = n.child_by_field_name("right") {
            self.walk(r, scope, under_defined);
        }
    }

    fn walk_for(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        let pattern = n.child_by_field_name("pattern");
        if let Some(pat) = pattern
            && pat.kind() == "identifier"
        {
            let name = self.text(pat).to_string();
            self.record_write(
                scope,
                &name,
                Write::at(pat, WriteKind::ForVar, None),
                IntroKind::Binding,
            );
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if pattern.map(|p| p.id()) == Some(child.id()) {
                continue;
            }
            self.walk(child, scope, under_defined);
        }
    }

    fn walk_rescue(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        // bind the exception variable FIRST: the handler body reads it
        let var = n.child_by_field_name("variable");
        self.bind_rescue_var(var, scope);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if var.map(|v| v.id()) == Some(child.id()) {
                continue;
            }
            self.walk(child, scope, under_defined);
        }
    }

    fn bind_rescue_var(&mut self, var: Option<Node>, scope: ScopeId) {
        if let Some(v) = var
            && let Some(ident) = v.children(&mut v.walk()).find(|c| c.kind() == "identifier")
        {
            let name = self.text(ident).to_string();
            self.record_write(
                scope,
                &name,
                Write::at(ident, WriteKind::RescueVar, None),
                IntroKind::Binding,
            );
        }
    }
}

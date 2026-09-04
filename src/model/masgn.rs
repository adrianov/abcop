//! Multiple-assignment target lists: `a, b = f` and destructuring forms.
//! Every bound name is a write of kind [`WriteKind::Masgn`] with no RHS.

use tree_sitter::Node;

use super::builder::Builder;
use super::{IntroKind, ScopeId, Write, WriteKind};

impl Builder<'_> {
    pub(super) fn collect_masgn_targets(&mut self, list: Node, scope: ScopeId) {
        let mut cursor = list.walk();
        for child in list.children(&mut cursor) {
            match child.kind() {
                "identifier" => self.bind_masgn_target(child, scope),
                "rest_assignment" | "destructured_left_assignment_list" => {
                    self.collect_masgn_targets_inner(child, scope);
                }
                _ => {}
            }
        }
    }

    fn collect_masgn_targets_inner(&mut self, list: Node, scope: ScopeId) {
        let mut cursor = list.walk();
        for child in list.children(&mut cursor) {
            if child.kind() == "identifier" {
                self.bind_masgn_target(child, scope);
            } else if child.named_child_count() > 0 && child.kind() != "integer" {
                self.collect_masgn_targets_inner(child, scope);
            }
        }
    }

    fn bind_masgn_target(&mut self, ident: Node, scope: ScopeId) {
        let name = self.text(ident).to_string();
        self.record_write(
            scope,
            &name,
            Write::at(ident, WriteKind::Masgn, None),
            IntroKind::Binding,
        );
    }
}

//! Read-recording constructs: unary operators (`defined?` taints), calls
//! (safe-nav site collection), and bare identifiers (reads or vcalls).

use tree_sitter::Node;

use super::builder::Builder;
use super::{Read, ScopeId};

impl Builder<'_> {
    /// Returns true when `kind` is a read-shaped construct.
    pub(super) fn walk_read(
        &mut self,
        n: Node,
        kind: &str,
        scope: ScopeId,
        under_defined: bool,
    ) -> bool {
        match kind {
            "unary" => {
                self.walk_unary(n, scope, under_defined);
                true
            }
            "call" => {
                self.walk_call(n, scope, under_defined);
                true
            }
            "identifier" => {
                self.walk_identifier(n, scope, under_defined);
                true
            }
            _ => false,
        }
    }

    fn walk_unary(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        let op_node = n.child_by_field_name("operator");
        let op = op_node.map(|o| self.text(o)).unwrap_or("");
        let ud = under_defined || op == "defined?";
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if op_node.map(|o| o.id()) == Some(child.id()) {
                continue;
            }
            self.walk(child, scope, ud);
        }
    }

    fn walk_call(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        // never treat the @method slot as a variable read
        let method_slot = n.child_by_field_name("method");
        self.note_csend_site(n, scope);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if method_slot.map(|m| m.id()) == Some(child.id()) {
                continue;
            }
            self.walk(child, scope, under_defined);
        }
    }

    /// Safe-navigation on a local receiver: recorded for the ABC
    /// repeated-csend discount.
    fn note_csend_site(&mut self, n: Node, scope: ScopeId) {
        let op = n
            .child_by_field_name("operator")
            .map(|o| self.text(o))
            .unwrap_or("")
            .to_string();
        if op == "&."
            && let Some(recv) = n.child_by_field_name("receiver")
            && recv.kind() == "identifier"
        {
            let name = self.text(recv);
            if self.lookup(scope, recv.start_byte(), name).is_some() {
                self.csend_sites.push((recv.start_byte(), name.into(), scope));
            }
        }
    }

    fn walk_identifier(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        let name = self.text(n).to_string();
        let r = Read {
            byte: n.start_byte(),
            under_defined,
        };
        if self.lookup(scope, r.byte, &name).is_some() {
            if !name.starts_with('_') {
                self.record_read(scope, &name, r);
            }
        } else {
            // unresolved bare identifier == zero-arity method call
            self.vcall_sites.push(n.start_byte());
        }
    }
}

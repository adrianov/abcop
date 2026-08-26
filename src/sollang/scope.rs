//! Collector walk for Solidity: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! Solidity notes: assignments to names no visible binding introduced
//! target state variables -- Solidity has no undeclared locals either,
//! so such targets contribute operand reads only; member `@property`
//! slots are never variable reads.

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, Scope, Write, child_of_kind};

use super::decl;

/// Static description of the Solidity walk.
static SPEC: Spec = Spec {
    skip_kinds: &[
        "parameter",
        "state_variable_declaration",
        "struct_declaration",
        "enum_declaration",
        "event_definition",
        "error_definition",
        "using_directive",
        "pragma_directive",
        "import_directive",
    ],
    block_scoped: &["block_statement", "unchecked_block"],
    function_kinds: &[
        "function_definition",
        "constructor_definition",
        "modifier_definition",
    ],
    exclude_fields: &[("member_expression", "property")],
    read_kinds: &["identifier"],
};

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<Scope> {
    let mut c = Collector {
        src,
        model: Model::rooted(),
    };
    dispatch(&mut c, root, 0);
    c.model.scopes
}

struct Collector<'a> {
    src: &'a [u8],
    model: Model,
}

impl Backend for Collector<'_> {
    fn spec(&self) -> &'static Spec {
        &SPEC
    }

    fn model(&mut self) -> &mut Model {
        &mut self.model
    }

    fn text_of(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn custom(&mut self, n: Node, scope: usize) {
        match n.kind() {
            // `uint256 x = expr;` and tuple heads `(bool ok, ) = ...`
            "variable_declaration_statement" => {
                decl::bind_declaration_statement(self, n, scope);
            }
            "assignment_expression" => self.walk_assignment(n, scope),
            // augmented assignment reads-and-rewrites a visible local;
            // member/index/state operands fall through to generic walking
            "augmented_assignment_expression" => match decl::plain_identifier_target(n) {
                Some(target) => {
                    self.rebind_local(target, scope, false, None);
                }
                None => self.walk_children(n, scope),
            },
            // i++ / --i read and rewrite
            "update_expression" => {
                if let Some(operand) = child_of_kind(n, "identifier") {
                    self.rebind_local(operand, scope, false, None);
                }
            }
            _ => self.walk_children(n, scope),
        }
    }
}

impl Collector<'_> {
    /// Plain `=` rebinds every visible local named by the head (tuple
    /// heads expand per declared name; only single-target assignments
    /// link an inlineable RHS). Compound operators rewrite-and-read.
    /// Assignments to names no visible binding introduced target state
    /// variables -- Solidity has no undeclared locals -- so their
    /// operands are reads only.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain = decl::top_level_op(n, self.src) == "=";
        let Some(left) = left else { return };

        let mut targets: Vec<Node> = Vec::new();
        decl::plain_identifier_targets(left, &mut targets);

        if targets.is_empty() {
            // state mapping / member write: operands are reads only
            self.walk_children(left, scope);
        } else {
            self.rebind_tuple(&targets, plain, right, scope);
        }

        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }

    fn rebind_tuple(&mut self, targets: &[Node], plain: bool, right: Option<Node>, scope: usize) {
        let single_pair = targets.len() == 1;
        for t in targets {
            let rhs = if plain && single_pair {
                right.map(|r| r.id())
            } else {
                None
            };
            let w = Write::assign(t.start_byte(), t.id(), rhs);
            self.bind_var(*t, scope, w, IntroKind::Assign);
            if !plain {
                self.read_at(scope, *t, t.end_byte());
            }
        }
    }

    fn read_at(&mut self, scope: usize, name_node: Node, byte: usize) {
        let name = self.text_of(name_node).to_string();
        self.model.record_read(scope, &name, byte);
    }
}

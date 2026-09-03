//! Collector walk for Zig: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! Zig notes: the grammar aliases both `const`/`var` declarations and
//! in-block assignment statements as `variable_declaration`. Container-
//! level state binds at Root (left unreported). Assignments to
//! non-locals (fields, derefs) contribute operand reads only. Payload
//! heads (`|x|`) and parameters are protocol.

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, Scope, Write};

use super::decl;

/// Static description of the Zig walk.
static SPEC: Spec = Spec {
    skip_kinds: &[
        "comment",
        "container_field",
        "function_signature",
        "enum_declaration",
        "error_set_declaration",
        "opaque_declaration",
        "using_namespace_declaration",
        "block_label",
        "break_label",
    ],
    block_scoped: &["block"],
    function_kinds: &[
        "function_declaration",
        "test_declaration",
        "comptime_declaration",
    ],
    exclude_fields: &[("field_expression", "member")],
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

pub(super) struct Collector<'a> {
    pub(super) src: &'a [u8],
    pub(super) model: Model,
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
            "variable_declaration" => decl::walk_var_decl(self, n, scope),
            "assignment_expression" => decl::walk_assignment(self, n, scope),
            "parameter" => self.bind_parameter(n, scope),
            "payload" => self.bind_payload(n, scope),
            _ => self.walk_children(n, scope),
        }
    }
}

impl Collector<'_> {
    /// Bind a named parameter; type annotations still walk for any
    /// nested expression reads.
    fn bind_parameter(&mut self, n: Node, scope: usize) {
        if let Some(name) = n.child_by_field_name("name") {
            self.bind_var(
                name,
                scope,
                Write::rewrite(name.start_byte(), name.id()),
                IntroKind::Binding,
            );
            self.walk_children_excluding_field(n, scope, "name");
        } else {
            self.walk_children(n, scope);
        }
    }

    /// `|a, *b|` capture heads: protocol bindings, never inline
    /// candidates.
    fn bind_payload(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "identifier" {
                self.bind_var(
                    child,
                    scope,
                    Write::rewrite(child.start_byte(), child.id()),
                    IntroKind::Binding,
                );
            }
        }
    }
}

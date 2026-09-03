//! Collector walk for Java: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! Java notes: member name slots (`field_access`'s `@field`,
//! `method_invocation`'s `@name`) and package-qualified type names are
//! never variable reads; enhanced-for heads and catch/resource bindings
//! are protocol, never candidates; braces inside `switch` groups share
//! the enclosing function scope via Block resolution.
//!
//! The kind-dispatch arms live in [`arms`].

mod arms;

use tree_sitter::Node;

use crate::scope_model::{IntroKind, Model, Scope, ScopeKind, Write};

/// Subtrees carrying no local-variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "formal_parameters",
    "field_declaration",
    "import_declaration",
    "package_declaration",
    "scoped_identifier",
];

/// Expressions whose given named fields are member references, not
/// variables: walking skips exactly those slots.
const EXCLUDE_FIELDS: &[(&str, &str)] = &[
    ("method_invocation", "name"),
    ("field_access", "field"),
    ("nullsafe_member_call_expression", "name"),
];

/// Kinds that open a nested scope: blocks, lambdas and switch blocks
/// capture outward; everything else stops resolution.
const BLOCK_SCOPED: &[&str] = &["block", "lambda_expression", "switch_block"];

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<Scope> {
    let mut c = Collector {
        src,
        model: Model::rooted(),
    };
    c.walk(root, 0);
    c.model.scopes
}

struct Collector<'a> {
    src: &'a [u8],
    model: Model,
}

impl Collector<'_> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn bind_var(&mut self, name_node: Node, scope: usize, w: Write, intro: IntroKind) {
        self.model
            .bind(scope, &self.text(name_node).to_string(), w, intro);
    }

    /// Bind a `variable_declarator`'s `@name`, optionally linking its
    /// `@value` as the inlinable RHS.
    fn bind_declarator(&mut self, n: Node, scope: usize, allow_rhs: bool) {
        if let Some(name) = n.child_by_field_name("name")
            && name.kind() == "identifier"
        {
            let rhs = if allow_rhs {
                n.child_by_field_name("value").map(|v| v.id())
            } else {
                None
            };

            self.bind_var(
                name,
                scope,
                Write::assign(name.start_byte(), name.id(), rhs),
                IntroKind::Assign,
            );
        }
    }

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if self.shortcut(n, scope, kind) {
            return;
        }
        if BLOCK_SCOPED.contains(&kind) {
            let s = self.model.open_scope(ScopeKind::Block, scope);
            self.walk_children(n, s);
            return;
        }
        self.arm(n, scope, kind);
    }
}

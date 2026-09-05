//! Swift backend for the shared `scope_model` walk.
//!
//! Nominal type bodies (`class_body` / `enum_class_body` / `protocol_body`)
//! are namespaces, not local-variable scopes: nested `func` / `init` /
//! `lambda_literal` / computed properties capture outer locals, so those
//! units open [`ScopeKind::Block`] and escape to root. Type members
//! (`property_declaration` directly under a type body) are not locals --
//! UsedOnce / NeverUsed ignore them -- but their initializers and
//! computed / observer bodies are still walked for nested locals and
//! captures. `include_root_scope` stays false (file-level bindings may be
//! consumed in other files).

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, ScopeKind, Write};

/// Type-body kinds whose direct `property_declaration` children are members,
/// not locals (tree-sitter-swift uses `class_body` for class/struct/actor/
/// extension; enums and protocols have their own body kinds).
const TYPE_BODIES: &[&str] = &["class_body", "enum_class_body", "protocol_body"];

// Grammar note: kinds must match tree-sitter-swift (`lambda_literal`, not
// a JS-style `closure_expression`; no `struct_body` / `statement_block`).
static SWIFT_SPEC: Spec = Spec {
    skip_kinds: &["import_declaration"],
    block_scoped: &[
        "class_body",
        "enum_class_body",
        "protocol_body",
        "function_declaration",
        "init_declaration",
        "lambda_literal",
        "computed_property",
        "computed_getter",
        "computed_setter",
        "computed_modify",
        "willset_didset_block",
        "catch_block",
    ],
    // Swift's functions/init/closures are closures w.r.t. outer locals.
    function_kinds: &[],
    read_kinds: &["simple_identifier"],
    // Member-access identifiers (`self.x`, `Type.m`, chained `a.b.c`) are
    // `simple_identifier` kind -- the same token Swift uses for real local
    // reads -- so they must NOT record phantom reads. Their role is conveyed
    // by field name (`suffix`/`name`), handled in `custom` below because
    // Swift routes `navigation_suffix` through `custom` before dispatch sees
    // it.
    exclude_fields: &[],
};

pub(super) fn swift_collect(root: Node, src: &[u8]) -> Vec<crate::scope_model::Scope> {
    let mut c = SwiftCollector {
        src,
        model: Model::rooted(),
    };
    dispatch(&mut c, root, 0);
    c.model.scopes
}

struct SwiftCollector<'a> {
    src: &'a [u8],
    model: Model,
}

impl Backend for SwiftCollector<'_> {
    fn spec(&self) -> &'static Spec {
        &SWIFT_SPEC
    }

    fn model(&mut self) -> &mut Model {
        &mut self.model
    }

    fn text_of(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn custom(&mut self, n: Node, scope: usize) {
        match n.kind() {
            // `let`/`var` declaration: `property_declaration` wraps a
            // `value_binding_pattern` plus optional `@value` / `@computed_value`.
            // Type members are not locals; function/computed locals bind.
            "property_declaration" => self.swift_bind(n, scope),
            // Member access `expr.member`: the `suffix` field holds the
            // member-name `simple_identifier`; skip it so `self.x` /
            // `Type.m` don't record a phantom read of a same-named local.
            "navigation_suffix" => self.walk_children_excluding_field(n, scope, "suffix"),
            // Swift's `assignment` node covers plain `=` (rebinds a
            // visible local) and compound operators (`+=`...) which
            // rewrite-and-read; compound binds as `Binding`, never an
            // inline candidate (see `candidates`).
            "assignment" => self.walk_assignment(n, scope),
            // for-in head `@item` is protocol (not a local introduction we
            // track); open a Block so body locals stay loop-scoped and walk
            // `@collection` / body via dispatch (a block_scoped boundary
            // would route the collection identifier through `custom` and
            // miss the read).
            "for_statement" => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                self.walk_children_excluding_field(n, s, "item");
            }
            _ => self.walk_children(n, scope),
        }
    }
}

impl SwiftCollector<'_> {
    /// Bind a local `property_declaration`, or walk a type member without
    /// introducing it. Always walks initializer / computed / observer
    /// subtrees (excluding the bound name) so nested locals and reads of
    /// outer locals are recorded.
    fn swift_bind(&mut self, n: Node, scope: usize) {
        if !is_type_member(n) {
            // `@name` is a `pattern` node wrapping a `bound_identifier`
            // `simple_identifier`; walk its named children to find the name.
            if let Some(name) = n
                .child_by_field_name("name")
                .and_then(|p| p.named_children(&mut p.walk()).next())
            {
                self.bind_var(
                    name,
                    scope,
                    Write::assign(
                        name.start_byte(),
                        name.id(),
                        n.child_by_field_name("value").map(|v| v.id()),
                    ),
                    IntroKind::Assign,
                );
            }
        }
        self.walk_children_excluding_field(n, scope, "name");
    }

    /// Plain `=` rebinds a visible local; compound assignment operators
    /// rewrite-and-read. Operands of assignments to unbound names are
    /// reads only (globals). Swift identifier bindings are
    /// `simple_identifier`.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        // Swift's `assignment` node fields are `target`/`result`; JS-family
        // uses `left`/`right` -- accept either field-name set.
        let left = n
            .child_by_field_name("left")
            .or_else(|| n.child_by_field_name("target"));
        let right = n
            .child_by_field_name("right")
            .or_else(|| n.child_by_field_name("result"));
        let plain = n
            .child_by_field_name("operator")
            .map_or(false, |o| self.text_of(o) == "=");
        if let Some(left) = left {
            if left.kind() == "simple_identifier" {
                self.rebind_local(left, scope, plain, right.map(|r| r.id()));
            } else {
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }
}

fn is_type_member(n: Node) -> bool {
    n.parent()
        .is_some_and(|p| TYPE_BODIES.iter().any(|k| p.kind() == *k))
}

//! Swift backend for the shared `scope_model` walk.
//!
//! Swift mirrors the JS/TS family: nominal type bodies (`class_body`/
//! `struct_body`) are namespaces, not function boundaries -- nested
//! `func`/`init`/closures capture outer `let` bindings -- so every unit
//! opens a `Block` scope that escapes to root and `include_root_scope`
//! stays false (module-level bindings may be consumed in other files).

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, Write};

// Swift's static scope/spec tables. Swift mirrors the JS/TS family:
// nominal type bodies (`class_body`/`struct_body`) are namespaces, not
// function boundaries -- nested `func`/`init`/closures capture outer
// `let` bindings -- so every unit opens a `Block` scope that escapes to
// root and `include_root_scope` stays false (module-level bindings may
// be consumed in other files).
static SWIFT_SPEC: Spec = Spec {
    skip_kinds: &["import_declaration"],
    block_scoped: &[
        "statement_block",
        "class_body",
        "struct_body",
        "enum_case_block",
        "function_declaration",
        "init_declaration",
        "closure_expression",
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
            // `value_binding_pattern` plus a `@value` initializer. Bind
            // the pattern's bound identifier, linking the init as RHS.
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
            _ => self.walk_children(n, scope),
        }
    }
}

impl SwiftCollector<'_> {
    /// Bind a `property_declaration`: introduce the bound identifier in its
    /// `@name` pattern, linking the initializer (`@value`) as the inlinable
    /// RHS when present.
    fn swift_bind(&mut self, n: Node, scope: usize) {
        // `@name` is a `pattern` node wrapping a `bound_identifier`
        // `simple_identifier`; walk its named children to find the name.
        let name = n
            .child_by_field_name("name")
            .and_then(|p| p.named_children(&mut p.walk()).next());
        let Some(name) = name else {
            return;
        };

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
        if let Some(value) = n.child_by_field_name("value") {
            dispatch(self, value, scope);
        }
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

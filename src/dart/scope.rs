//! Collector walk for Dart: the language-specific arms. Generic
//! dispatch (skip subtrees, member-slot exclusions, Block scopes,
//! plain identifier reads) lives in [`crate::scope_model::walk`].
//!
//! Dart notes: Dart has no undeclared locals, so an assignment whose
//! target is not a visible local writes a field/outer symbol -- the
//! operands contribute reads only. Field and top-level declarations bind
//! state that never participates in local tracking: their initializers
//! still produce reads. Parameters, for-in heads and catch bindings are
//! protocol. Pattern binding lives in [`super::patterns`].

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, ScopeKind, Write, child_of_kind};

static SPEC: Spec = Spec {
    skip_kinds: &[
        "comment",
        "block_comment",
        "documentation_block_comment",
        // named-argument labels and goto-style statement labels are pure
        // names in both roles
        "label",
        // `this.x` / `super.x` forwarding: fields, not locals
        "constructor_param",
        "super_formal_parameter",
        "escape_sequence",
        // abstract/external headers carry no body to walk
        "external_function_declaration",
        "external_getter_declaration",
        "external_setter_declaration",
        "external_variable_declaration",
    ],
    block_scoped: &[
        "block",
        "function_declaration",
        "method_declaration",
        "getter_declaration",
        "setter_declaration",
        "local_function_declaration",
        "function_expression",
    ],
    function_kinds: &[],
    read_kinds: &["identifier", "identifier_dollar_escaped"],
    exclude_fields: &[
        ("member_expression", "property"),
        ("null_aware_member_expression", "property"),
        ("cascade_member_expression", "property"),
        ("cascade_call_expression", "property"),
        ("cascade_null_aware_member_expression", "property"),
        ("cascade_selector", "property"),
        ("annotation", "name"),
        ("enum_constant", "name"),
        ("function_signature", "name"),
        ("getter_signature", "name"),
        ("setter_signature", "name"),
        // each constructor family reuses `name` for the class token and
        // the `.named` member; excluding kills both phantoms
        ("constructor_signature", "name"),
        ("factory_constructor_signature", "name"),
        ("constant_constructor_signature", "name"),
        ("redirecting_factory_constructor_signature", "name"),
    ],
};

/// Signature-only node kinds that make a bare `declaration` (a bodyless
/// constructor header like `Foo.named(this.x);`) parameter-bearing.
const CTOR_SIG_KINDS: &[&str] = &[
    "constructor_signature",
    "factory_constructor_signature",
    "constant_constructor_signature",
    "redirecting_factory_constructor_signature",
];

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<crate::scope_model::Scope> {
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
            // `var x = ...` / `int b = a++`: name + optional value field
            "initialized_variable_definition" => self.bind_declarator_with_rhs_field(n, scope),
            // plain/compound assignments to bare identifiers
            "assignment_expression" => self.walk_assignment(n, scope),
            // default-value expressions must keep producing their reads
            "formal_parameter" => self.bind_parameter(n, scope),
            // class/file state: the name slot binds nothing local -- but
            // initializer expressions are real expression contexts
            "initialized_identifier" | "static_final_declaration" | "field_initializer" => {
                if let Some(v) = n.child_by_field_name("value") {
                    dispatch(self, v, scope);
                }
            }
            // irrefutable `final [p, q] = list;`
            "pattern_variable_declaration" => self.walk_pattern_declaration(n, scope),
            // destructuring assignment `[a, b] = pair;`
            "pattern_assignment" => self.walk_pattern_assignment(n, scope),
            // one shared block over all clauses so catch exception and
            // stack-trace names scope across their handler blocks
            "try_statement" => self.walk_try(n, scope),
            // bodyless constructor headers need their own scope so their
            // parameters never leak into the enclosing context
            "declaration" if CTOR_SIG_KINDS.iter().any(|k| child_of_kind(n, k).is_some()) => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                self.walk_children(n, s);
            }
            _ => self.walk_children(n, scope),
        }
    }
}

impl Collector<'_> {
    /// Bind a declared parameter; the remaining children (type, default
    /// value) walk on with the name slot suppressed.
    fn bind_parameter(&mut self, n: Node, scope: usize) {
        let Some(name) = n.child_by_field_name("name") else {
            return;
        };
        let w = Write::rewrite(name.start_byte(), name.id());
        self.bind_var(name, scope, w, IntroKind::Binding);
        self.walk_children_excluding_field(n, scope, "name");
    }

    fn walk_try(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        let mut caught = Vec::new();
        let mut cursor = n.walk();
        for clause in n
            .children(&mut cursor)
            .filter(|c| c.kind() == "catch_clause")
        {
            for field in ["exception", "stack_trace"] {
                if let Some(id) = clause.child_by_field_name(field) {
                    let w = Write::rewrite(id.start_byte(), id.id());
                    self.bind_var(id, s, w, IntroKind::Binding);
                    caught.push(id.id());
                }
            }
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if caught.contains(&child.id()) {
                continue;
            }
            dispatch(self, child, s);
        }
    }
}

/// The bare identifier behind an `assignable_expression`, if the target
/// is a plain variable rather than an index/member form.
pub(super) fn bare_target(left: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = left.walk();
    let named: Vec<_> = left
        .children(&mut cursor)
        .filter(|c| c.is_named())
        .collect();
    match named[..] {
        [only] if only.kind() == "identifier" => Some(only),
        _ => None,
    }
}

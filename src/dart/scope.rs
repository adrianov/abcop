//! Collector walk for Dart: the language-specific arms. Generic dispatch
//! (skip subtrees, member-slot exclusions, Block scopes, identifier
//! reads) lives in [`crate::scope_model::walk`].
//!
//! Dart has no undeclared locals, so assignments to non-locals write
//! fields/outer state -- operand reads only. Field/top-level initializers
//! still produce reads; parameters, for-in heads and catch bindings are
//! protocol. Pattern binding lives in [`super::patterns`].

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, ScopeKind, Write, child_of_kind};

static SPEC: Spec = Spec {
    skip_kinds: &[
        // comments; named-argument and goto-style statement labels are
        // pure names in both roles; `this.x`/`super.x` forwarding are
        // fields, not locals; string escape internals
        "comment",
        "block_comment",
        "documentation_block_comment",
        "label",
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
        self.dispatch_custom(n, scope);
    }
}

impl Collector<'_> {
    fn dispatch_custom(&mut self, n: Node, scope: usize) {
        if self.dispatch_binding(n, scope) || self.dispatch_structure(n, scope) {
            return;
        }
        self.walk_children(n, scope);
    }

    fn dispatch_binding(&mut self, n: Node, scope: usize) -> bool {
        match n.kind() {
            "initialized_variable_definition" => {
                self.bind_declarator_with_rhs_field(n, scope);
                self.walk_children_excluding_field(n, scope, "name");
            }
            "assignment_expression" => self.walk_assignment(n, scope),
            "formal_parameter" if !self.is_protocol_param(n) => self.bind_parameter(n, scope),
            "formal_parameter" => {}
            "initialized_identifier" | "static_final_declaration" | "field_initializer" => {
                self.walk_state_initializer(n, scope)
            }
            "pattern_variable_declaration" => self.walk_pattern_declaration(n, scope),
            "pattern_assignment" => self.walk_pattern_assignment(n, scope),
            _ => return false,
        }
        true
    }

    fn dispatch_structure(&mut self, n: Node, scope: usize) -> bool {
        match n.kind() {
            "try_statement" => self.walk_try(n, scope),
            "declaration" if is_ctor_header(n) => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                self.walk_children(n, s);
            }
            _ => return false,
        }
        true
    }

    /// Redirecting factory formals and abstract `throw UnimplementedError()`
    /// stubs declare protocol names: generated code or overrides consume
    /// them, so single-file analysis must not treat them as dead locals.
    fn is_protocol_param(&self, param: Node) -> bool {
        in_redirecting_factory_signature(param)
            || enclosing_callable(param).is_some_and(|n| throws_unimplemented(&self, n))
    }

    /// Bind a declared parameter; the remaining children (type, default
    /// value) walk on with the name slot suppressed.
    fn bind_parameter(&mut self, n: Node, scope: usize) {
        let Some(name) = n.child_by_field_name("name") else {
            return;
        };

        self.bind_var(
            name,
            scope,
            Write::rewrite(name.start_byte(), name.id()),
            IntroKind::Binding,
        );
        self.walk_children_excluding_field(n, scope, "name");
    }

    /// Field/top-level state initializer: the name slot binds nothing,
    /// the value expression still produces its reads.
    fn walk_state_initializer(&mut self, n: Node, scope: usize) {
        if let Some(v) = n.child_by_field_name("value") {
            dispatch(self, v, scope);
        }
    }

    fn walk_try(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        let caught = self.bind_catch_names(n, s);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if caught.contains(&child.id()) {
                continue;
            }
            dispatch(self, child, s);
        }
    }

    /// Bind catch exception/stack-trace names into the shared try scope;
    /// returns their ids so the sibling walk skips re-processing them.
    fn bind_catch_names(&mut self, try_node: Node<'_>, scope: usize) -> Vec<usize> {
        let mut caught = Vec::new();
        for field in ["exception", "stack_trace"] {
            let mut cursor = try_node.walk();
            for clause in try_node
                .children(&mut cursor)
                .filter(|c| c.kind() == "catch_clause")
            {
                if let Some(id) = clause.child_by_field_name(field) {
                    self.bind_var(
                        id,
                        scope,
                        Write::rewrite(id.start_byte(), id.id()),
                        IntroKind::Binding,
                    );
                    caught.push(id.id());
                }
            }
        }
        caught
    }
}

/// Wraps a signature-only constructor header?
fn is_ctor_header(declaration: Node<'_>) -> bool {
    CTOR_SIG_KINDS
        .iter()
        .any(|k| child_of_kind(declaration, k).is_some())
}

fn in_redirecting_factory_signature(mut param: Node<'_>) -> bool {
    while let Some(parent) = param.parent() {
        if parent.kind() == "redirecting_factory_constructor_signature" {
            return true;
        }
        param = parent;
    }
    false
}

fn enclosing_callable(mut param: Node<'_>) -> Option<Node<'_>> {
    while let Some(parent) = param.parent() {
        match parent.kind() {
            "method_declaration" | "function_declaration" => return Some(parent),
            "source_file" | "program" => return None,
            _ => param = parent,
        }
    }
    None
}

fn throws_unimplemented(c: &Collector<'_>, callable: Node<'_>) -> bool {
    callable
        .child_by_field_name("body")
        .and_then(|body| body.named_child(0))
        .filter(|n| n.kind() == "throw_expression")
        .and_then(|n| n.child_by_field_name("value"))
        .and_then(|value| value.child_by_field_name("function"))
        .is_some_and(|func| func.kind() == "identifier" && c.text_of(func) == "UnimplementedError")
}

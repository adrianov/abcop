//! Scope-introducing constructs: method/class bodies, blocks and lambdas.

use tree_sitter::Node;

use super::builder::{body_of, Builder};
use super::{ScopeId, ScopeKind};

impl Builder<'_> {
    /// Enter a fresh scope and walk only its parameters and body.
    /// Returns true when `kind` introduced a scope.
    pub(super) fn walk_scope_intro(&mut self, n: Node, kind: &str, parent: ScopeId) -> bool {
        match kind {
            "method" | "singleton_method" => {
                let s = self.scope_for(n, ScopeKind::Method, None);
                self.declare_params_and_body(n, s);
            }
            "class" | "module" | "singleton_class" => {
                let s = self.scope_for(n, ScopeKind::ClassLike, None);
                if let Some(body) = n.child_by_field_name("body") {
                    self.walk(body, s, false);
                }
            }
            "block" | "do_block" | "lambda" => {
                let s = self.scope_for(n, ScopeKind::Block, Some(parent));
                self.declare_block_params_and_body(n, s);
            }
            _ => return false,
        }
        true
    }

    fn declare_params_and_body(&mut self, n: Node, s: ScopeId) {
        if let Some(p) = n.child_by_field_name("parameters") {
            self.declare_params(p, s);
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.walk(body, s, false);
        }
    }

    fn declare_block_params_and_body(&mut self, n: Node, s: ScopeId) {
        if let Some(p) = n.child_by_field_name("parameters") {
            self.declare_params(p, s); // block params always shadow
        }
        if let Some(body) = body_of(n) {
            self.walk(body, s, false);
        }
    }

    pub(super) fn declare_params(&mut self, container: Node, scope: ScopeId) {
        let mut cursor = container.walk();
        for child in container.children(&mut cursor) {
            match child.kind() {
                "," | "(" | ")" | "|" => {}
                "identifier" | "optional_parameter" | "keyword_parameter" | "block_parameter"
                | "splat_parameter" => self.declare_named_param(child, scope),
                _ => self.declare_wrapped_param(child, scope),
            }
        }
    }

    /// A named parameter binds directly; default/kw values may contain
    /// arbitrary expressions and are walked.
    fn declare_named_param(&mut self, child: Node, scope: ScopeId) {
        if let Some(v) = child.child_by_field_name("value") {
            self.walk(v, scope, false);
        }
        if let Some(name) = super::builder::declared_name(child, self.src)
            && !name.starts_with('_')
        {
            self.bind_entry(scope, name, child.start_byte());
        }
    }

    /// Splat wrappers, forwarding args, shadow params etc.: bind any
    /// identifier found inside.
    fn declare_wrapped_param(&mut self, child: Node, scope: ScopeId) {
        let mut sub = child.walk();
        for inner in child.children(&mut sub) {
            if inner.kind() == "identifier" {
                let name = self.text(inner).to_string();
                if !name.starts_with('_') {
                    self.bind_entry(scope, name, inner.start_byte());
                }
            }
        }
    }
}

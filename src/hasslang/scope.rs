//! Collector walk for Haskell: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! Haskell notes: value-level `function` / top-level `bind` open function
//! scopes. Local `let`/`where` binds are Assign intros. Do-binds,
//! generators, case alternatives and lambda patterns are protocol
//! Binding (UsedOnce/NeverUsed exempt via `exempt_bindings`). Type-level
//! `function` (`a -> b`) and signatures are skipped. Module-level names
//! bind at Root and stay unreported.

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, Scope, ScopeKind, Write};

use super::nodes::{
    arrow_rhs, bind_name, each_body_child, has_match, is_decl_list_child, match_expression,
};
use super::patterns::pattern_vars;

/// Static description of the Haskell walk. Function boundaries are
/// opened in [`Collector::custom`] because `function` also names the
/// type-level arrow form.
static SPEC: Spec = Spec {
    skip_kinds: &[
        "comment",
        "haddock",
        "pragma",
        "header",
        "imports",
        "import",
        "signature",
        "data_type",
        "newtype",
        "type_synomym",
        "type_family",
        "data_family",
        "default_types",
        "deriving_instance",
        "fixity",
        "foreign_import",
        "kind_signature",
        "default_signature",
        "pattern_synonym",
        "constructor",
        "operator",
        "module_id",
        "name",
    ],
    block_scoped: &[],
    function_kinds: &[],
    exclude_fields: &[],
    read_kinds: &["variable"],
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
            "function" if has_match(n) => self.walk_value_function(n, scope),
            "bind" => self.walk_bind(n, scope),
            "lambda" => self.walk_lambda(n, scope),
            "alternative" => self.walk_alternative(n, scope),
            "generator" | "pattern_guard" => self.walk_arrow_bind(n, scope),
            "class" => self.walk_decl_container(n, scope, "class_declarations"),
            "instance" => self.walk_decl_container(n, scope, "instance_declarations"),
            _ => self.walk_children(n, scope),
        }
    }
}

impl Collector<'_> {
    fn walk_bind(&mut self, n: Node, scope: usize) {
        if has_match(n) && is_decl_list_child(n) {
            self.walk_top_bind(n, scope);
        } else if has_match(n) {
            self.walk_local_bind(n, scope);
        } else {
            self.walk_do_bind(n, scope);
        }
    }

    fn walk_value_function(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Function, scope);
        if let Some(pats) = n.child_by_field_name("patterns") {
            self.bind_pattern(pats, s, IntroKind::Binding);
        }
        each_body_child(n, |child| {
            dispatch(self, child, s);
        });
    }

    fn walk_top_bind(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Function, scope);
        each_body_child(n, |child| {
            dispatch(self, child, s);
        });
    }

    fn walk_local_bind(&mut self, n: Node, scope: usize) {
        if let Some(name) = bind_name(n) {
            self.bind_var(
                name,
                scope,
                Write::assign(
                    name.start_byte(),
                    name.id(),
                    match_expression(n).map(|r| r.id()),
                ),
                IntroKind::Assign,
            );
        }
        each_body_child(n, |child| {
            dispatch(self, child, scope);
        });
    }

    fn walk_do_bind(&mut self, n: Node, scope: usize) {
        self.bind_arrow_head(n, scope);
        if let Some(expr) = arrow_rhs(n) {
            dispatch(self, expr, scope);
        }
    }

    fn walk_lambda(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Function, scope);
        if let Some(pats) = n.child_by_field_name("patterns") {
            self.bind_pattern(pats, s, IntroKind::Binding);
        }
        if let Some(expr) = n.child_by_field_name("expression") {
            dispatch(self, expr, s);
        }
    }

    fn walk_alternative(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        let p = n
            .child_by_field_name("pattern")
            .or_else(|| n.child_by_field_name("patterns"));
        if let Some(p) = p {
            self.bind_pattern(p, s, IntroKind::Binding);
        }
        each_body_child(n, |child| {
            dispatch(self, child, s);
        });
    }

    fn walk_arrow_bind(&mut self, n: Node, scope: usize) {
        self.bind_arrow_head(n, scope);
        if let Some(expr) = n.child_by_field_name("expression") {
            dispatch(self, expr, scope);
        }
    }

    /// `class` / `instance` expose declaration lists as named children,
    /// not fields.
    fn walk_decl_container(&mut self, n: Node, scope: usize, kind: &str) {
        for child in n.children(&mut n.walk()) {
            if child.kind() == kind {
                dispatch(self, child, scope);
            }
        }
    }

    fn bind_arrow_head(&mut self, n: Node, scope: usize) {
        if let Some(p) = n.child_by_field_name("pattern") {
            self.bind_pattern(p, scope, IntroKind::Binding);
            return;
        }
        if let Some(name) = bind_name(n) {
            self.bind_var(
                name,
                scope,
                Write::rewrite(name.start_byte(), name.id()),
                IntroKind::Binding,
            );
        }
    }

    fn bind_pattern(&mut self, pattern: Node, scope: usize, intro: IntroKind) {
        let mut ids = Vec::new();
        pattern_vars(pattern, self.src, &mut ids);
        for id in ids {
            self.bind_var(
                id,
                scope,
                Write::rewrite(id.start_byte(), id.id()),
                intro,
            );
        }
    }
}

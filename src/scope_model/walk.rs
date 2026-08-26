//! Shared collector dispatch for language backends. A backend describes
//! its generic behavior once ([`Spec`]) and implements three trivial
//! accessors ([`Backend`]); [`dispatch`] then consumes everything that
//! needs no language-specific judgment -- skip subtrees, member-slot
//! exclusions, Block scopes, Function boundaries, plain identifier
//! reads -- before handing genuinely custom node kinds back.

use tree_sitter::Node;

use super::{Model, ScopeKind};

/// Static description of a backend's generic walk behavior.
pub struct Spec {
    /// Subtrees carrying no local-variable writes or reads.
    pub skip_kinds: &'static [&'static str],
    /// Kinds that open a nested [`ScopeKind::Block`] scope.
    pub block_scoped: &'static [&'static str],
    /// Kinds that open a [`ScopeKind::Function`] boundary.
    pub function_kinds: &'static [&'static str],
    /// Expressions whose named fields are member references, not
    /// variables: walking skips exactly those slots.
    pub exclude_fields: &'static [(&'static str, &'static str)],
}

/// A backend collector driving the shared [`Model`].
pub trait Backend {
    fn spec(&self) -> &'static Spec;
    fn model(&mut self) -> &mut Model;
    fn text_of(&self, n: Node) -> &str;
    /// Language-specific arms. Everything the [`Spec`] covers has been
    /// consumed before this runs; fall back to walking children for the
    /// remainder.
    fn custom(&mut self, n: Node, scope: usize);

    /// Bind `name_node` (text used verbatim, underscore-filtered by the
    /// model) with the given write.
    fn bind_var(&mut self, name_node: Node, scope: usize, w: super::Write, intro: super::IntroKind) {
        let name = self.text_of(name_node).to_string();
        self.model().bind(scope, &name, w, intro);
    }

    /// Bind a `variable_declarator`'s `@name`, linking its initializer
    /// as the inlinable RHS when the grammar exposes one.
    fn bind_declarator_with_rhs_field(&mut self, n: Node, scope: usize) {
        if let Some(name) = n.child_by_field_name("name")
            && name.kind() == "identifier"
        {
            // grammars differ on exposing an initializer field; when
            // absent, it is the first named child after the `=` token
            let rhs = match n.child_by_field_name("value") {
                Some(v) => Some(v.id()),
                None => {
                    let mut c = n.walk();
                    let mut after_eq = false;
                    n.children(&mut c)
                        .find(|ch| {
                            if !ch.is_named() && self.text_of(*ch) == "=" {
                                after_eq = true;
                                false
                            } else {
                                after_eq && ch.is_named()
                            }
                        })
                        .map(|v| v.id())
                }
            };
            let w = super::Write::assign(name.start_byte(), name.id(), rhs);
            self.bind_var(name, scope, w, super::IntroKind::Assign);
        }
    }

}

/// Consume everything the [`Spec`] covers. Returns `true` when handled;
/// otherwise runs the backend's custom arms itself.
pub fn dispatch(b: &mut impl Backend, n: Node, scope: usize) -> bool {
    let kind = n.kind();
    let spec = b.spec();
    if spec.skip_kinds.contains(&kind) {
        return true;
    }
    if let Some((_, field)) = spec.exclude_fields.iter().find(|(k, _)| *k == kind)
        && let Some(excluded) = n.child_by_field_name(field).map(|c| c.id())
    {
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            if child.id() != excluded {
                b.custom(child, scope);
            }
        }
        return true;
    }
    if spec.block_scoped.contains(&kind) {
        let s = b.model().open_scope(ScopeKind::Block, scope);
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            b.custom(child, s);
        }
        return true;
    }
    if spec.function_kinds.contains(&kind) {
        let s = b.model().open_scope(ScopeKind::Function, scope);
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            b.custom(child, s);
        }
        return true;
    }
    if kind == "identifier" {
        let name = b.text_of(n).to_string();
        if !name.starts_with('_') {
            b.model().record_read(scope, &name, n.start_byte());
        }
        return true;
    }
    b.custom(n, scope);
    true
}

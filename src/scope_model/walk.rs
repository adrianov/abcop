//! Shared collector dispatch for language backends. A backend describes
//! its generic behavior once ([`Spec`]) and implements three trivial
//! accessors ([`Backend`]); [`dispatch`] then consumes everything that
//! needs no language-specific judgment -- skip subtrees, member-slot
//! exclusions, Block scopes, Function boundaries, plain identifier
//! reads -- before handing genuinely custom node kinds back.

use tree_sitter::Node;

use super::ScopeKind;
pub use super::backend::Backend;

/// Static description of a backend's generic walk behavior.
pub struct Spec {
    /// Subtrees carrying no local-variable writes or reads.
    pub skip_kinds: &'static [&'static str],
    /// Kinds that open a nested [`ScopeKind::Block`] scope.
    pub block_scoped: &'static [&'static str],
    /// Kinds that open a [`ScopeKind::Function`] boundary.
    pub function_kinds: &'static [&'static str],
    /// Named-reference kinds (per grammar) that record a variable read:
    /// JS/TS use `identifier`, Swift uses `simple_identifier`, etc.
    pub read_kinds: &'static [&'static str],
    /// Expressions whose named fields are member references, not
    /// variables: walking skips exactly those slots.
    pub exclude_fields: &'static [(&'static str, &'static str)],
}

/// Consume everything the [`Spec`] covers. Returns `true` when handled;
/// otherwise runs the backend's custom arms itself.
pub fn dispatch(b: &mut impl Backend, n: Node, scope: usize) -> bool {
    let kind = n.kind();
    let spec = b.spec();
    if spec.skip_kinds.contains(&kind) {
        return true;
    }
    if spec.read_kinds.contains(&kind) {
        record_read(b, n, scope);
        return true;
    }
    if walk_excluding_field_slot(b, n, scope, kind, spec) {
        return true;
    }
    match boundary_kind(spec, &kind) {
        Some(boundary) => {
            let s = b.model().open_scope(boundary, scope);
            custom_children(b, n, s);
        }
        None => b.custom(n, scope),
    }
    true
}

/// Record `n`'s text as a read of that name at its start byte.
fn record_read(b: &mut impl Backend, n: Node, scope: usize) {
    let name = b.text_of(n).to_string();
    b.model().record_read(scope, &name, n.start_byte());
}

/// Walk children while skipping the named-child slot this kind uses for
/// member/property names (`member_expression.name` et al) -- those name
/// rather than read. Returns false when the kind has no exclude entry or
/// the named slot is absent, letting the caller try later cases.
fn walk_excluding_field_slot(
    b: &mut impl Backend,
    n: Node,
    scope: usize,
    kind: &str,
    spec: &Spec,
) -> bool {
    let Some((_, field)) = spec.exclude_fields.iter().find(|(k, _)| *k == kind) else {
        return false;
    };
    let Some(excluded) = n.child_by_field_name(field).map(|c| c.id()) else {
        return false;
    };
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        if child.id() != excluded {
            b.custom(child, scope);
        }
    }
    true
}

/// The child-scope boundary this node opens, if any.
fn boundary_kind(spec: &Spec, kind: &str) -> Option<ScopeKind> {
    if spec.block_scoped.contains(&kind) {
        Some(ScopeKind::Block)
    } else if spec.function_kinds.contains(&kind) {
        Some(ScopeKind::Function)
    } else {
        None
    }
}

/// Route every child of `n` through the backend's custom arms -- the
/// shared traversal for scope boundaries whose opening/closing tokens
/// carry no semantics of their own.
fn custom_children(b: &mut impl Backend, n: Node, scope: usize) {
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        b.custom(child, scope);
    }
}

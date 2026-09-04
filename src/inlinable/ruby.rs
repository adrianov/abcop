//! Ruby UsedOnce / NeverUsed RHS inlining.

use tree_sitter::Node;

use crate::model::FileModel;

use super::{immediate_substitutable, RUBY_IDENT, RUBY_UNITS};

/// Call/index trees (including nested in ternaries or interpolations) need
/// an immediate read; otherwise pure compositions — ternaries, interpolated
/// strings, stable locals, ivars.
pub fn ruby_inlinable_rhs(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
    write_site: Option<Node>,
) -> bool {
    if has_effect(fm, n, scope) {
        return match read_byte {
            None => true,
            Some(rb) => write_site.is_some_and(|site| immediate_substitutable(site, rb)),
        };
    }
    is_pure(fm, n, scope, write_byte, read_byte)
}

const LITERALS: &[&str] = &[
    "integer",
    "float",
    "true",
    "false",
    "nil",
    "simple_symbol",
    "symbol",
    "hash_key_symbol",
    "self",
    "constant",
    "instance_variable",
    "class_variable",
    "global_variable",
    "string_content",
    "escape_sequence",
];

const COMPOSE: &[&str] = &[
    "string",
    "array",
    "range",
    "binary",
    "conditional",
    "interpolation",
    "scope_resolution",
    "delimited_symbol",
    "hash",
    "pair",
];

fn has_effect(fm: &FileModel, n: Node, scope: usize) -> bool {
    // `defined?` is never a call-effect; purity rejects it separately.
    if defined_unary(fm, n) {
        return false;
    }
    let kind = n.kind();
    if RUBY_UNITS.contains(&kind) {
        return true;
    }
    if kind == RUBY_IDENT {
        return is_vcall(fm, n, scope);
    }
    named(n).into_iter().any(|c| has_effect(fm, c, scope))
}

fn defined_unary(fm: &FileModel, n: Node) -> bool {
    n.kind() == "unary"
        && n.child_by_field_name("operator")
            .is_some_and(|o| fm.text(o) == "defined?")
}

fn is_vcall(fm: &FileModel, n: Node, scope: usize) -> bool {
    let name = fm.text(n);
    !matches!(name, "__FILE__" | "__LINE__" | "__ENCODING__")
        && fm.lookup(scope, n.start_byte(), name).is_none()
}

fn is_pure(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
) -> bool {
    let kind = n.kind();
    if LITERALS.contains(&kind) {
        return true;
    }
    if kind == RUBY_IDENT {
        return ident_ok(fm, n, scope, write_byte, read_byte);
    }
    if COMPOSE.contains(&kind) {
        return all_pure(fm, n, scope, write_byte, read_byte);
    }
    if kind == "parenthesized_statements" {
        return paren_ok(fm, n, scope, write_byte, read_byte);
    }
    kind == "unary" && !defined_unary(fm, n) && all_pure(fm, n, scope, write_byte, read_byte)
}

fn ident_ok(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
) -> bool {
    match read_byte {
        Some(end) => alias_stable(fm, scope, fm.text(n), write_byte, end),
        None => true,
    }
}

fn paren_ok(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
) -> bool {
    let inner = named(n);
    inner.len() == 1 && is_pure(fm, inner[0], scope, write_byte, read_byte)
}

fn alias_stable(
    fm: &FileModel,
    scope: usize,
    name: &str,
    write_byte: usize,
    read_byte: usize,
) -> bool {
    match fm.lookup(scope, write_byte, name) {
        None => true,
        Some(bind_scope) => fm.scopes[bind_scope]
            .entries
            .get(name)
            .is_none_or(|entry| {
                !entry
                    .writes
                    .iter()
                    .any(|w| w.byte > write_byte && w.byte < read_byte)
            }),
    }
}

fn all_pure(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
) -> bool {
    named(n)
        .into_iter()
        .all(|c| is_pure(fm, c, scope, write_byte, read_byte))
}

fn named<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    n.children(&mut n.walk()).filter(|c| c.is_named()).collect()
}

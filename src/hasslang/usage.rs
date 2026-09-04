//! Haskell candidate-evaluation policy over the shared scope model: the
//! RHS purity whitelist and the straight-line veto/boundary lists.

use tree_sitter::Node;

use super::HsFile;
use crate::inlinable::{HS_IDENT, HS_UNITS};
use crate::scope_model;

/// Ancestors that mark a write as conditional.
const VETO_KINDS: &[&str] = &[
    "conditional",
    "case",
    "alternative",
    "guards",
    "guard",
    "boolean",
    "multi_way_if",
    "list_comprehension",
];

/// Ancestors that end the straight-line check (unit boundaries).
/// Local `bind` must not appear here — it would mask an enclosing
/// conditional/case veto (Zig keeps only function-like owners).
const OWNER_KINDS: &[&str] = &["function", "lambda"];

static HS_SEMANTICS: scope_model::Semantics = scope_model::Semantics {
    pure,
    unit_kinds: HS_UNITS,
    ident_kind: HS_IDENT,
    veto: VETO_KINDS,
    owners: OWNER_KINDS,
    include_root_scope: false,
    exempt_bindings: true,
};

pub fn used_once_offenses(fm: &HsFile) -> Vec<crate::used_once::UsedOnceOffense> {
    scope_model::used_once_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &HS_SEMANTICS,
    )
}

pub fn never_used_offenses(fm: &HsFile) -> Vec<crate::never_used::NeverUsedOffense> {
    scope_model::never_used_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &HS_SEMANTICS,
    )
}

/// Conservative RHS purity: literals and operator compositions over
/// them. Applications and variable references fail through children.
fn pure(n: Node) -> bool {
    match n.kind() {
        "literal" | "integer" | "float" | "char" | "string" => true,
        "parens" | "infix" | "negation" | "list" | "tuple" => children_pure(n),
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    n.children(&mut n.walk())
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

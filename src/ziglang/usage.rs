//! Zig candidate-evaluation policy over the shared scope model: the
//! RHS purity whitelist and the straight-line veto/boundary lists.

use tree_sitter::Node;

use super::ZigFile;
use crate::inlinable::{ZIG_IDENT, ZIG_UNITS};
use crate::scope_model;

/// Ancestors that mark a write as conditional.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "if_expression",
    "for_statement",
    "for_expression",
    "while_statement",
    "while_expression",
    "switch_expression",
    "switch_case",
    "catch_expression",
    "errdefer_statement",
];

/// Ancestors that end the straight-line check (unit boundaries).
const OWNER_KINDS: &[&str] = &[
    "function_declaration",
    "test_declaration",
    "comptime_declaration",
];

static ZIG_SEMANTICS: scope_model::Semantics = scope_model::Semantics {
    pure,
    unit_kinds: ZIG_UNITS,
    ident_kind: ZIG_IDENT,
    veto: VETO_KINDS,
    owners: OWNER_KINDS,
    include_root_scope: false,
};

pub fn used_once_offenses(fm: &ZigFile) -> Vec<crate::used_once::UsedOnceOffense> {
    scope_model::used_once_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &ZIG_SEMANTICS,
    )
}

pub fn never_used_offenses(fm: &ZigFile) -> Vec<crate::never_used::NeverUsedOffense> {
    scope_model::never_used_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &ZIG_SEMANTICS,
    )
}

/// Conservative RHS purity: literals and operator compositions over
/// them. Calls and identifier references fail through their children.
fn pure(n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "character" | "string" | "multiline_string" | "boolean" | "true"
        | "false" | "null" | "undefined" => true,
        "parenthesized_expression" | "binary_expression" | "unary_expression" => children_pure(n),
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    n.children(&mut n.walk())
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

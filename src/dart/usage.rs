//! Dart candidate-evaluation policy over the shared scope model: the
//! RHS purity whitelist and the straight-line veto/boundary lists.

use tree_sitter::Node;

use super::DartFile;
use crate::inlinable::{DART_IDENT, DART_UNITS};
use crate::scope_model;

/// Ancestors that mark a write as conditional.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "do_statement",
    "switch_statement",
    "switch_expression",
    "try_statement",
    "catch_clause",
];

/// Ancestors that end the straight-line check: the unit roots and the
/// lambda body boundary.
const OWNER_KINDS: &[&str] = &[
    "function_declaration",
    "method_declaration",
    "getter_declaration",
    "setter_declaration",
    "local_function_declaration",
    "function_expression",
];

static DART_SEMANTICS: scope_model::Semantics = scope_model::Semantics {
    pure,
    unit_kinds: DART_UNITS,
    ident_kind: DART_IDENT,
    veto: VETO_KINDS,
    owners: OWNER_KINDS,
    include_root_scope: false,
};

pub fn used_once_offenses(fm: &DartFile) -> Vec<crate::used_once::UsedOnceOffense> {
    scope_model::used_once_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &DART_SEMANTICS,
    )
}

pub fn never_used_offenses(fm: &DartFile) -> Vec<crate::never_used::NeverUsedOffense> {
    scope_model::never_used_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &DART_SEMANTICS,
    )
}

/// Conservative RHS purity: literals (including strings whose
/// interpolations are themselves literal-only), collections of literals,
/// and operator compositions over them. Identifier references, calls and
/// interpolated variables fail through their children.
fn pure(n: Node) -> bool {
    match n.kind() {
        "decimal_integer_literal"
        | "decimal_floating_point_literal"
        | "hex_integer_literal"
        | "true"
        | "false"
        | "null_literal" => true,
        // interpolated strings carry template_substitution children and
        // fail through them; raw strings never interpolate
        "string_literal" => children_pure(n),
        "raw_string_literal_double_quotes"
        | "raw_string_literal_single_quotes"
        | "raw_string_literal_double_quotes_multiple"
        | "raw_string_literal_single_quotes_multiple" => true,
        // multi-line / raw wrappers compose string parts
        "template_chars_single"
        | "template_chars_single_single"
        | "template_chars_double"
        | "template_chars_double_single"
        | "template_chars_raw_slash"
        | "escape_sequence" => true,
        // `${...}` references runtime values: not pure
        "template_substitution" => false,
        "list_literal" | "set_or_map_literal" | "record_literal" => children_pure(n),
        "parenthesized_expression"
        | "additive_expression"
        | "multiplicative_expression"
        | "bitwise_and_expression"
        | "bitwise_or_expression"
        | "bitwise_xor_expression"
        | "shift_expression"
        | "if_null_expression"
        | "unary_expression" => children_pure(n),
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    n.children(&mut n.walk())
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

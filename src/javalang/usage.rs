//! Java candidate-evaluation policy over the shared scope model: the
//! RHS purity whitelist and the straight-line veto/boundary lists.

use tree_sitter::Node;

use super::JavaFile;
use crate::inlinable::{JAVA_IDENT, JAVA_UNITS};
use crate::scope_model;

/// Ancestors that mark a write as conditional.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "enhanced_for_statement",
    "while_statement",
    "do_statement",
    "switch_expression",
    "switch_statement",
    "try_statement",
    "try_with_resources_statement",
    "catch_clause",
];

/// Ancestors that end the straight-line check (unit boundaries).
const OWNER_KINDS: &[&str] = &["method_declaration", "constructor_declaration"];

static JAVA_SEMANTICS: scope_model::Semantics = scope_model::Semantics {
    pure: pure,
    unit_kinds: JAVA_UNITS,
    ident_kind: JAVA_IDENT,
    veto: VETO_KINDS,
    owners: OWNER_KINDS,
    include_root_scope: true,
    exempt_bindings: false,
};

pub fn used_once_offenses(fm: &JavaFile) -> Vec<crate::used_once::UsedOnceOffense> {
    scope_model::used_once_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &JAVA_SEMANTICS,
    )
}

pub fn never_used_offenses(fm: &JavaFile) -> Vec<crate::never_used::NeverUsedOffense> {
    scope_model::never_used_offenses(
        fm.tree.root_node(),
        fm.src,
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &JAVA_SEMANTICS,
    )
}

/// Conservative RHS purity: literals and operator compositions over
/// them; references to other locals, calls and array creations are
/// rejected, mirroring the Rust backend.
fn pure(n: Node) -> bool {
    match n.kind() {
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal"
        | "decimal_floating_point_literal"
        | "char_literal"
        | "string_literal"
        | "boolean_literal"
        | "null_literal" => true,
        "parenthesized_expression" | "binary_expression" | "unary_expression" => children_pure(n),
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    n.children(&mut n.walk())
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

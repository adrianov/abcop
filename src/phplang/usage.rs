//! PHP candidate-evaluation policy over the shared scope model: the
//! RHS purity whitelist and the straight-line veto/boundary lists.

use tree_sitter::Node;

use super::PhpFile;
use crate::scope_model;

/// Ancestors that mark a write as conditional.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "while_statement",
    "do_statement",
    "for_statement",
    "foreach_statement",
    "switch_statement",
    "try_statement",
    "catch_clause",
    "match_expression",
];

/// Ancestors that end the straight-line check (unit boundaries).
const OWNER_KINDS: &[&str] = &["function_definition", "method_declaration"];

static PHP_SEMANTICS: scope_model::Semantics = scope_model::Semantics {
    pure: pure,
    veto: VETO_KINDS,
    owners: OWNER_KINDS,
    include_root_scope: false,
};

pub fn used_once_offenses(fm: &PhpFile) -> Vec<crate::used_once::UsedOnceOffense> {
    scope_model::used_once_offenses(
        fm.tree.root_node(),
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &PHP_SEMANTICS,
    )
}

pub fn never_used_offenses(fm: &PhpFile) -> Vec<crate::never_used::NeverUsedOffense> {
    scope_model::never_used_offenses(&|byte| fm.line_col(byte), &fm.scopes, &PHP_SEMANTICS)
}

/// Conservative RHS purity: literals, arrays of literals, and operator
/// compositions over them. Interpolated strings reference variables and
/// are rejected via their children.
fn pure(n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "string" | "encapsed_string" => children_pure(n),
        "boolean" | "null" => true,
        "array_creation_expression" | "array_element_initializer"
        | "parenthesized_expression" | "binary_expression" | "unary_op_expression" => {
            children_pure(n)
        }
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

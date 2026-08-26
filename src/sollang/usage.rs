//! Solidity candidate-evaluation policy over the shared scope model:
//! the RHS purity whitelist and the straight-line veto/boundary lists.

use tree_sitter::Node;

use super::SolFile;
use crate::scope_model;

/// Ancestors that mark a write as conditional.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "do_while_statement",
    "try_statement",
    "catch_clause",
];

/// Ancestors that end the straight-line check (unit boundaries).
const OWNER_KINDS: &[&str] = &[
    "function_definition",
    "constructor_definition",
    "modifier_definition",
];

static SOLIDITY_SEMANTICS: scope_model::Semantics = scope_model::Semantics {
    pure,
    veto: VETO_KINDS,
    owners: OWNER_KINDS,
    include_root_scope: true,
};

pub fn used_once_offenses(fm: &SolFile) -> Vec<crate::used_once::UsedOnceOffense> {
    scope_model::used_once_offenses(
        fm.tree.root_node(),
        &|byte| fm.line_col(byte),
        &fm.scopes,
        &SOLIDITY_SEMANTICS,
    )
}

pub fn never_used_offenses(fm: &SolFile) -> Vec<crate::never_used::NeverUsedOffense> {
    scope_model::never_used_offenses(&|byte| fm.line_col(byte), &fm.scopes, &SOLIDITY_SEMANTICS)
}

/// Conservative RHS purity: literals, arrays/tuples of literals, and
/// operator compositions over them. The grammar wraps every operand in
/// an `expression` node, so unwrapping is part of the whitelist; calls
/// inside that wrapper fail it naturally.
fn pure(n: Node) -> bool {
    match n.kind() {
        "number_literal" | "string_literal" | "string" | "boolean_literal" | "true" | "false" => {
            true
        }
        "expression"
        | "parenthesized_expression"
        | "tuple_expression"
        | "array_expression"
        | "binary_expression"
        | "unary_op_expression"
        | "unary_expression" => children_pure(n),
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

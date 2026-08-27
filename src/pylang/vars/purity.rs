//! Purity and placement gates applied to a single-use candidate before
//! it can be reported.

use tree_sitter::Node;

use super::VETO_KINDS;

/// Conservative RHS purity: literals and operator compositions over them.
/// References to other locals are rejected, mirroring the Rust backend.
pub(super) fn pure(n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "true" | "false" | "none" => true,
        "string" => children_pure(n),
        "string_content" | "escape_sequence" => true,
        "list"
        | "tuple"
        | "set"
        | "dictionary"
        | "pair"
        | "unary_operator"
        | "binary_operator"
        | "boolean_operator"
        | "parenthesized_expression" => children_pure(n),
        _ => false,
    }
}

fn children_pure(n: Node) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(ch))
}

/// Straight-line execution check up to the nearest scope boundary.
pub(super) fn unconditionally_executed(write_node: Node) -> bool {
    const OWNERS: [&str; 3] = ["function_definition", "class_definition", "lambda"];
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if VETO_KINDS.contains(&n.kind()) {
            return false;
        }
        if OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

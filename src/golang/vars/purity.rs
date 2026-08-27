//! Conservative RHS analysis and straight-line execution checks for
//! candidate inline writes.

use std::collections::HashMap;

use tree_sitter::Node;

/// Control-flow heads: a write beneath any of these may never be inlined.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "expression_switch_statement",
    "type_switch_statement",
    "select_statement",
];

/// Literal kinds an expression may fold from without touching a local.
const PURE_LITERALS: &[&str] = &[
    "int_literal",
    "float_literal",
    "imaginary_literal",
    "rune_literal",
    "raw_string_literal",
    "interpreted_string_literal",
    "true",
    "false",
    "nil",
    "iota",
];

/// Conservative RHS purity: literals and operator compositions over them.
/// References to other locals, calls and composite literals are rejected,
/// mirroring the Rust backend.
pub(super) fn pure(n: Node) -> bool {
    match n.kind() {
        k if PURE_LITERALS.contains(&k) => true,
        "parenthesized_expression" => n.named_child(0).map(pure).unwrap_or(false),
        "binary_expression" => operands_pure(n, 0),
        "unary_expression" => operands_pure(n, 1),
        _ => false,
    }
}

/// All named children from offset `skip` on are pure. Unary expressions
/// skip their operator token; binary operators are consumed together with
/// their operands.
fn operands_pure(n: Node, skip: usize) -> bool {
    let mut c = n.walk();
    n.children(&mut c).skip(skip).all(pure) && (skip == 0 || unary_op_ok(n))
}

/// Unary operators that keep an expression constant-foldable; `&` and
/// `<-` create references / channel receives and are rejected.
fn unary_op_ok(n: Node) -> bool {
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        if !ch.is_named() {
            return matches!(ch.utf8_text(b"").unwrap_or(""), "-" | "+" | "^" | "!");
        }
    }
    false
}

/// Straight-line execution check up to the nearest Function boundary;
/// bare blocks do not break straight-line execution.
pub(super) fn unconditionally_executed(write_node: Node) -> bool {
    const OWNERS: [&str; 2] = ["function_declaration", "method_declaration"];
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

/// Index every node by its id so writes can re-resolve RHS nodes later.
pub(super) fn index_nodes<'t>(root: Node<'t>) -> HashMap<usize, Node<'t>> {
    let mut map = HashMap::new();
    rec(root, &mut map);
    map
}

fn rec<'t>(n: Node<'t>, map: &mut HashMap<usize, Node<'t>>) {
    map.insert(n.id(), n);
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        rec(child, map);
    }
}

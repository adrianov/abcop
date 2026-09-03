//! Solidity declaration-statement binding: `uint256 x = expr;` heads and
//! tuple declarations `(bool ok, ) = ...`, plus the operator-text helper
//! for grammars that carry compound operators as bare punctuator children
//! instead of an `operator` field.

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, dispatch};
use crate::scope_model::{IntroKind, Write};

/// Bind every declared name of a `variable_declaration_statement`. A
/// single-declarator statement links the initializer as an inlinable RHS;
/// tuple declarations bind each element without one. The initializer
/// subtree is still walked for its own effects (calls may mutate state).
pub(super) fn bind_declaration_statement(b: &mut impl Backend, n: Node, scope: usize) {
    let (decls, value) = split_head(n);
    let single_pair = decls.len() == 1 && value.is_some();
    for d in &decls {
        if let Some(name) = d.child_by_field_name("name") {
            let rhs = if single_pair {
                value.map(|v| v.id())
            } else {
                None
            };

            b.bind_var(
                name,
                scope,
                Write::assign(name.start_byte(), name.id(), rhs),
                IntroKind::Assign,
            );
        }
    }
    if let Some(v) = value {
        dispatch(b, v, scope);
    }
}

/// Gather a statement head's declared names plus its initializer -- the
/// sole named child that is neither a declaration nor a terminator.
fn split_head<'t>(n: Node<'t>) -> (Vec<Node<'t>>, Option<Node<'t>>) {
    let mut decls = Vec::new();
    let mut value = None;
    for child in n.children(&mut n.walk()) {
        match child.kind() {
            "variable_declaration" | "variable_declaration_tuple" => {
                collect_decls(child, &mut decls);
            }
            ";" | "=" => {}
            _ if child.is_named() => value = Some(child),
            _ => {}
        }
    }
    (decls, value)
}

/// First unnamed child's text -- Solidity exposes no `operator` field, so
/// the punctuator token itself carries `=` vs `+=` et al.
pub(super) fn top_level_op<'t>(n: Node<'t>, src: &'t [u8]) -> &'t str {
    n.children(&mut n.walk())
        .find(|ch| !ch.is_named())
        .map(|ch| ch.utf8_text(src).unwrap_or(""))
        .unwrap_or("")
}

/// Identifier targets bound by an assignment head. Tuple heads expand
/// per declared name; member/array/call targets reference their
/// operands instead and bind nothing.
pub(super) fn plain_identifier_targets<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    match n.kind() {
        "identifier" => out.push(n),
        "member_expression" | "array_access" | "call_expression" => {}
        _ => {
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                plain_identifier_targets(child, out);
            }
        }
    }
}

/// The single identifier target of a non-tuple assignment head, if any.
pub(super) fn plain_identifier_target(n: Node) -> Option<Node> {
    let mut out = Vec::new();
    plain_identifier_targets(n, &mut out);
    if out.len() == 1 { out.pop() } else { None }
}

fn collect_decls<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    match n.kind() {
        "variable_declaration" => out.push(n),
        "variable_declaration_tuple" => {
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                collect_decls(child, out);
            }
        }
        _ => {}
    }
}

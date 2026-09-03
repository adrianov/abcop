//! Conservative RHS analysis and straight-line execution checks for
//! candidate inline writes.

use std::collections::HashMap;

use tree_sitter::Node;

use crate::inlinable::{GO_IDENT, GO_UNITS, immediate_substitutable, keep_init_kind};

use super::{Scope, ScopeKind};

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

pub(super) fn inlinable_rhs(
    src: &[u8],
    scopes: &[Scope],
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
    write_site: Option<Node>,
) -> bool {
    if GO_UNITS.contains(&n.kind()) {
        return match read_byte {
            None => true,
            Some(rb) => write_site.is_some_and(|site| immediate_substitutable(site, rb)),
        };
    }
    if n.kind() == GO_IDENT {
        return match read_byte {
            Some(end) => alias_stable(
                scopes,
                scope,
                write_byte,
                n.utf8_text(src).unwrap_or(""),
                write_byte,
                end,
            ),
            None => true,
        };
    }
    pure(n)
}

pub(super) fn keep_init(n: Node) -> bool {
    keep_init_kind(n, GO_UNITS)
}

fn alias_stable(
    scopes: &[Scope],
    scope: usize,
    pos: usize,
    name: &str,
    write_byte: usize,
    read_byte: usize,
) -> bool {
    let Some(bind_scope) = lookup(scopes, scope, pos, name) else {
        return true;
    };
    let Some(entry) = scopes[bind_scope].entries.get(name) else {
        return true;
    };
    !entry
        .writes
        .iter()
        .any(|w| w.byte > write_byte && w.byte < read_byte)
}

fn lookup(scopes: &[Scope], scope: usize, pos: usize, name: &str) -> Option<usize> {
    let data = &scopes[scope];
    if let Some(e) = data.entries.get(name) {
        return if e.intro_byte <= pos {
            Some(scope)
        } else {
            None
        };
    }
    match data.kind {
        ScopeKind::Block => lookup(scopes, data.parent?, pos, name),
        _ => None,
    }
}

/// Literals and operator compositions over them.
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
    n.children(&mut n.walk()).skip(skip).all(pure) && (skip == 0 || unary_op_ok(n))
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

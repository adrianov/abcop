//! Purity and placement gates applied to a single-use candidate before
//! it can be reported.

use tree_sitter::Node;

use crate::inlinable::{PY_IDENT, PY_UNITS, immediate_substitutable, keep_init_kind};

use super::Scope;
use super::VETO_KINDS;

/// Literals and operator compositions over them.
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

pub(super) fn inlinable_rhs(
    src: &[u8],
    scopes: &[Scope],
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
    write_site: Option<Node>,
) -> bool {
    if PY_UNITS.contains(&n.kind()) {
        return match read_byte {
            None => true,
            Some(rb) => write_site.is_some_and(|site| immediate_substitutable(site, rb)),
        };
    }
    if n.kind() == PY_IDENT {
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
    keep_init_kind(n, PY_UNITS)
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
        super::ScopeKind::Block => lookup(scopes, data.parent?, pos, name),
        _ => None,
    }
}

fn children_pure(n: Node) -> bool {
    n.children(&mut n.walk())
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

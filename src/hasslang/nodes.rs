//! Shared Haskell AST helpers: names, matches, decl-list placement.

use tree_sitter::Node;

const DECL_LISTS: &[&str] = &[
    "declarations",
    "class_declarations",
    "instance_declarations",
];

pub(super) fn has_match(n: Node<'_>) -> bool {
    n.children(&mut n.walk()).any(|c| c.kind() == "match")
}

pub(super) fn is_decl_list_child(n: Node<'_>) -> bool {
    n.parent()
        .is_some_and(|p| DECL_LISTS.contains(&p.kind()))
}

/// Value-level function (has a `match`) or module/class/instance bind.
pub(super) fn is_unit(n: Node<'_>) -> bool {
    match n.kind() {
        "function" => has_match(n),
        "bind" => has_match(n) && is_decl_list_child(n),
        _ => false,
    }
}

pub(super) fn bind_name(n: Node<'_>) -> Option<Node<'_>> {
    n.child_by_field_name("name").or_else(|| {
        n.children(&mut n.walk())
            .find(|c| c.kind() == "variable" || c.kind() == "prefix_id")
    })
}

pub(super) fn match_expression(n: Node<'_>) -> Option<Node<'_>> {
    n.children(&mut n.walk()).find_map(|c| {
        (c.kind() == "match")
            .then(|| c.child_by_field_name("expression"))
            .flatten()
    })
}

/// Walk `match` / `local_binds` children of a decl or alternative.
pub(super) fn each_body_child<'t>(n: Node<'t>, mut f: impl FnMut(Node<'t>)) {
    for child in n.children(&mut n.walk()) {
        if matches!(child.kind(), "match" | "local_binds") {
            f(child);
        }
    }
}

/// Expression after `<-` / `←`, or the `@expression` field when present.
pub(super) fn arrow_rhs(n: Node<'_>) -> Option<Node<'_>> {
    if let Some(expr) = n.child_by_field_name("expression") {
        return Some(expr);
    }
    let mut past = false;
    for child in n.children(&mut n.walk()) {
        if !child.is_named() && matches!(child.kind(), "<-" | "←") {
            past = true;
            continue;
        }
        if past && child.is_named() {
            return Some(child);
        }
    }
    None
}

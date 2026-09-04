//! `let … in …` adjacency for call-chain UsedOnce (Haskell and similar).

use tree_sitter::Node;

/// `let binds in body`: a call-chain RHS may move into `body` when the
/// write lives in `binds` and the read *is* that body (not merely nested
/// inside a larger `in` expression with intervening work).
pub(super) fn substitutable(write_site: Node, read_site: Node) -> bool {
    let Some(let_in) = ancestor_kind(write_site, "let_in") else {
        return false;
    };
    let Some(binds) = let_in.child_by_field_name("binds") else {
        return false;
    };
    let Some(body) = let_in.child_by_field_name("expression") else {
        return false;
    };
    under_node(write_site, binds) && peeled_is(body, read_site)
}

fn peeled_is(mut body: Node, read_site: Node) -> bool {
    while body.named_child_count() == 1
        && matches!(body.kind(), "parens" | "expression" | "exp")
    {
        body = body.named_child(0).expect("named_child_count == 1");
    }
    body.id() == read_site.id()
}

fn ancestor_kind<'t>(mut n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    while let Some(p) = n.parent() {
        if p.kind() == kind {
            return Some(p);
        }
        n = p;
    }
    None
}

fn under_node(mut n: Node, ancestor: Node) -> bool {
    loop {
        if n.id() == ancestor.id() {
            return true;
        }
        match n.parent() {
            Some(p) => n = p,
            None => return false,
        }
    }
}

//! Unit-name extraction for Dart AbcSize offenses.
//!
//! Dart's grammar reuses the `name` field across every signature family
//! (plain functions, getters/setters and all four constructor flavors),
//! and the constructor ones apply it to *both* the class token and the
//! `.named` member. The rule here is therefore source order: the last
//! `name`-field identifier inside a signature wins. Parameter lists,
//! type parameters and initializer lists terminate descent -- their name
//! slots are protocol or field references, never the unit's own label.

use tree_sitter::Node;

/// Signature subtrees consulted for a unit's declared name.
const NAME_SIG_KINDS: &[&str] = &[
    "method_signature",
    "function_signature",
    "getter_signature",
    "setter_signature",
    "constructor_signature",
    "factory_constructor_signature",
    "constant_constructor_signature",
    "redirecting_factory_constructor_signature",
];

pub(super) fn unit_name<'t>(unit: Node<'t>, src: &'t [u8]) -> String {
    let mut names = Vec::new();
    let mut cursor = unit.walk();
    for child in unit.children(&mut cursor) {
        if !NAME_SIG_KINDS.contains(&child.kind()) {
            continue;
        }
        collect_sig_names(child, &mut names);
    }
    if let Some(n) = names.into_iter().next_back() {
        return n.utf8_text(src).unwrap_or("").to_string();
    }
    // operator members (`operator ==`) declare no name field; the
    // signature label is the stable short name
    match desc_of_kind(unit, "operator_signature") {
        Some(sig) => sig.utf8_text(src).unwrap_or("<operator>").to_string(),
        None => "<operator>".to_string(),
    }
}

fn collect_sig_names<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    if n.kind() == "formal_parameter_list"
        || n.kind() == "type_parameters"
        || n.kind() == "initializers"
    {
        return;
    }
    if n.kind() == "identifier"
        && n.parent()
            .is_some_and(|p| p.field_name_for_child(index_in_parent(p, n)) == Some("name"))
    {
        out.push(n);
    }
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        collect_sig_names(ch, out);
    }
}

fn index_in_parent(parent: Node<'_>, child: Node<'_>) -> u32 {
    let mut c = parent.walk();
    parent
        .children(&mut c)
        .position(|ch| ch.id() == child.id())
        .unwrap_or(0) as u32
}

/// First descendant of the given kind, document order.
pub(super) fn desc_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = n.walk();
    n.children(&mut cursor).find_map(|c| {
        if c.kind() == kind {
            return Some(c);
        }
        desc_of_kind(c, kind)
    })
}

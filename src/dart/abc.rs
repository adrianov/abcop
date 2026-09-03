//! AbcSize over Dart trees: unit discovery, unit naming and offense
//! assembly.
//!
//! Units are top-level `function_declaration`s plus class-level members
//! (`method_declaration` covering plain methods, getters/setters and all
//! constructor flavors, and the file-level getter/setter declarations).
//! Anonymous function-likes (`function_expression`, local functions) roll
//! into the enclosing unit, mirroring Ruby blocks / Rust closures.
//! Counting itself lives in [`super::tally`]; naming is here because a
//! discovered unit is worthless without its label.

use tree_sitter::Node;

use super::DartFile;
use super::tally::{Tally, UNIT_KINDS};
use crate::abc::{AbcOffense, offense_at};

pub(crate) fn all_scores(fm: &DartFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm.tree.root_node(), fm.src, &mut |unit, name| {
        let Some(body) = unit.child_by_field_name("body") else {
            return;
        };
        let mut t = Tally {
            src: fm.src,
            ..Default::default()
        };
        t.walk(body);
        offenses.push(offense_at(unit, name, t.a, t.b, t.c));
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &str)) {
    if UNIT_KINDS.contains(&n.kind()) {
        f(n, &unit_name(n, src));
        return;
    }
    for child in n.children(&mut n.walk()) {
        visit_units(child, src, f);
    }
}

// ---------------------------------------------------------------------------
// Unit naming
//
// Dart's grammar reuses the `name` field across every signature family
// (plain functions, getters/setters and all four constructor flavors),
// and the constructor ones apply it to *both* the class token and the
// `.named` member. The rule here is therefore source order: the last
// `name`-field identifier inside a signature wins. Parameter lists,
// type parameters and initializer lists terminate descent -- their name
// slots are protocol or field references, never the unit's own label.

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

fn unit_name<'t>(unit: Node<'t>, src: &'t [u8]) -> String {
    let mut names = Vec::new();
    let mut cursor = unit.walk();
    for child in unit.children(&mut cursor) {
        if NAME_SIG_KINDS.contains(&child.kind()) {
            collect_sig_names(child, &mut names);
        }
    }
    if let Some(n) = names.into_iter().next_back() {
        return n.utf8_text(src).unwrap_or("").to_string();
    }
    operator_label(unit, src)
}

/// Operator members (`operator ==`) declare no name field; the
/// signature label is the stable short name.
fn operator_label<'t>(unit: Node<'t>, src: &'t [u8]) -> String {
    match desc_of_kind(unit, "operator_signature") {
        Some(sig) => sig.utf8_text(src).unwrap_or("<operator>").to_string(),
        None => "<operator>".into(),
    }
}

/// Slot kinds whose descent terminates: parameters, type parameters and
/// initializer lists never carry the unit's own label.
fn is_nameless_slot(kind: &str) -> bool {
    matches!(
        kind,
        "formal_parameter_list" | "type_parameters" | "initializers"
    )
}

fn collect_sig_names<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    if is_nameless_slot(n.kind()) {
        return;
    }
    if n.kind() == "identifier" && has_name_slot(n) {
        out.push(n);
    }
    for ch in n.children(&mut n.walk()) {
        collect_sig_names(ch, out);
    }
}

/// Is this node the value of its parent's `name` field?
fn has_name_slot(n: Node<'_>) -> bool {
    n.parent()
        .is_some_and(|p| p.field_name_for_child(index_in_parent(p, n)) == Some("name"))
}

fn index_in_parent(parent: Node<'_>, child: Node<'_>) -> u32 {
    parent
        .children(&mut parent.walk())
        .position(|ch| ch.id() == child.id())
        .unwrap_or(0) as u32
}

/// First descendant of the given kind, document order.
fn desc_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    n.children(&mut n.walk()).find_map(|c| {
        if c.kind() == kind {
            return Some(c);
        }
        desc_of_kind(c, kind)
    })
}

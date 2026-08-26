//! AbcSize over Dart trees: unit discovery and offense assembly.
//!
//! Units are top-level `function_declaration`s plus class-level members
//! (`method_declaration` covering plain methods, getters/setters and all
//! constructor flavors, and the file-level getter/setter declarations).
//! Anonymous function-likes (`function_expression`, local functions) roll
//! into the enclosing unit, mirroring Ruby blocks / Rust closures. The
//! counting itself lives in [`super::tally`].

use tree_sitter::Node;

use super::DartFile;
use super::names::unit_name;
use super::tally::Tally;
use crate::abc::{AbcOffense, fmt_vector};

/// Unit kinds; also the tally boundary set.
const UNIT_KINDS: &[&str] = &[
    "function_declaration",
    "method_declaration",
    "getter_declaration",
    "setter_declaration",
];

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
        offenses.push(offense(unit, name, &t));
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

/// Assemble one finished offense, positioned at its unit root.
fn offense(unit: Node<'_>, name: &str, t: &Tally<'_>) -> AbcOffense {
    let pos = unit.start_position();
    let raw = ((t.a * t.a + t.b * t.b + t.c * t.c) as f64).sqrt();
    AbcOffense {
        line: pos.row + 1,
        end_line: unit.end_position().row + 1,
        column: pos.column,
        name: name.to_string(),
        score: (raw * 100.0).round() / 100.0,
        vector: fmt_vector(t.a, t.b, t.c),
    }
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &str)) {
    if UNIT_KINDS.contains(&n.kind()) {
        let name = unit_name(n, src);
        f(n, &name);
        return;
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(child, src, f);
    }
}

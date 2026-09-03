//! AbcSize over Zig trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::ZigFile;
use super::decl::{ignored_name, var_decl_a_count};
use crate::abc::{AbcOffense, offense_at};

const UNIT_KINDS: &[&str] = &[
    "function_declaration",
    "test_declaration",
    "comptime_declaration",
];

/// Binary operators counted toward C; arithmetic, bitwise and shifts
/// count toward B.
const C_OPERATORS: &[&str] = &["and", "or", "orelse", "==", "!=", "<", ">", "<=", ">="];

pub(crate) fn all_scores(fm: &ZigFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm.tree.root_node(), fm.src, &mut |unit, name| {
        if let Some(o) = unit_offense(fm, unit, name) {
            offenses.push(o);
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

fn unit_offense(fm: &ZigFile, unit: Node, name: &str) -> Option<AbcOffense> {
    let body = unit_body(unit)?;
    let mut t = Tally {
        src: fm.src,
        ..Default::default()
    };
    t.walk(body);
    Some(offense_at(unit, name, t.a, t.b, t.c))
}

fn unit_body(unit: Node<'_>) -> Option<Node<'_>> {
    unit.child_by_field_name("body").or_else(|| {
        unit.children(&mut unit.walk())
            .find(|c| c.kind() == "block")
    })
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &str)) {
    if UNIT_KINDS.contains(&n.kind()) {
        f(n, &unit_name(n, src));
    }
    for child in n.children(&mut n.walk()) {
        visit_units(child, src, f);
    }
}

fn unit_name(n: Node<'_>, src: &[u8]) -> String {
    if let Some(name) = n.child_by_field_name("name") {
        return name.utf8_text(src).unwrap_or("").to_string();
    }
    match n.kind() {
        "test_declaration" => test_label(n, src),
        "comptime_declaration" => "comptime".into(),
        _ => "<fn>".into(),
    }
}

fn test_label(n: Node<'_>, src: &[u8]) -> String {
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        if matches!(child.kind(), "string" | "identifier") {
            return child.utf8_text(src).unwrap_or("test").to_string();
        }
    }
    "test".into()
}

#[derive(Default)]
struct Tally<'s> {
    src: &'s [u8],
    a: u32,
    b: u32,
    c: u32,
}

impl Tally<'_> {
    fn op_of(&self, n: Node<'_>) -> Option<&str> {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
    }

    fn walk(&mut self, n: Node) {
        // Nested units are scored on their own visit; do not roll their
        // bodies into the enclosing unit.
        if UNIT_KINDS.contains(&n.kind()) {
            return;
        }
        self.tally_node(n);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }

    fn tally_node(&mut self, n: Node) {
        match n.kind() {
            "variable_declaration" => self.a += var_decl_a_count(n, self.src),
            "assignment_expression" => self.tally_assignment(n),
            "payload" => self.a += payload_a(n, self.src),
            "if_statement" | "if_expression" | "for_statement" | "for_expression"
            | "while_statement" | "while_expression" | "switch_case" | "catch_expression" => {
                self.c += 1
            }
            "binary_expression" => self.tally_binary(n),
            "call_expression" | "builtin_function" | "unary_expression" | "try_expression" => {
                self.b += 1
            }
            _ => {}
        }
    }

    fn tally_assignment(&mut self, n: Node) {
        let plain = self.op_of(n) == Some("=");
        let left = n.child_by_field_name("left");
        if left.is_some_and(|l| l.kind() == "identifier") {
            if left.is_some_and(|l| ignored_name(l, self.src)) {
                return;
            }
            self.a += 1;
            return;
        }
        if !plain {
            self.a += 1;
        }
    }

    fn tally_binary(&mut self, n: Node) {
        if self.op_of(n).is_some_and(|op| C_OPERATORS.contains(&op)) {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }
}

fn payload_a(n: Node<'_>, src: &[u8]) -> u32 {
    n.children(&mut n.walk())
        .filter(|ch| ch.kind() == "identifier" && !ignored_name(*ch, src))
        .count() as u32
}

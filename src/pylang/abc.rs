//! AbcSize over Python trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::PyFile;
use crate::abc::{AbcOffense, fmt_vector};

/// Subtrees that belong to another unit (nested defs/classes score
/// themselves). Lambdas are NOT boundaries: their single expression rolls
/// into the enclosing unit like Ruby blocks / Rust closures.
fn is_boundary(kind: &str) -> bool {
    matches!(kind, "function_definition" | "class_definition")
}

/// Kinds counted toward B: implicit and explicit calls plus operator
/// applications (Python operators are runtime-dispatched like Ruby sends).
const B_KINDS: &[&str] = &[
    "call",
    "attribute",
    "subscript",
    "binary_operator",
    "unary_operator",
    "not_operator",
    "interpolation",
];

/// Kinds counted toward C: branch points and condition logic.
const C_KINDS: &[&str] = &[
    "if_statement",
    "elif_clause",
    "while_statement",
    "for_statement",
    "for_in_clause",
    "conditional_expression",
    "boolean_operator",
    "comparison_operator",
    "except_clause",
    "case_clause",
    "if_clause",
];

pub(crate) fn all_scores(fm: &PyFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm.tree.root_node(), fm.src, &mut |unit, name| {
        let Some(body) = unit.child_by_field_name("body") else {
            return;
        };
        let mut t = Tally::default();
        t.walk(body);
        let pos = unit.start_position();
        let raw = ((t.a * t.a + t.b * t.b + t.c * t.c) as f64).sqrt();
        offenses.push(AbcOffense {
            line: pos.row + 1,
            end_line: unit.end_position().row + 1,
            column: pos.column,
            name: name.to_string(),
            score: (raw * 100.0).round() / 100.0,
            vector: fmt_vector(t.a, t.b, t.c),
        });
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

/// Find every named `function_definition` at any depth, including inside
/// other units' bodies.
fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &'t str)) {
    if n.kind() == "function_definition" {
        if let Some(name) = n.child_by_field_name("name") {
            if let Ok(text) = name.utf8_text(src) {
                f(n, text);
            }
        }
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(child, src, f);
    }
}

#[derive(Default)]
struct Tally {
    a: u32,
    b: u32,
    c: u32,
}

impl Tally {
    fn walk(&mut self, n: Node) {
        if is_boundary(n.kind()) {
            return;
        }
        match n.kind() {
            "assignment" => {
                if let Some(left) = n.child_by_field_name("left") {
                    self.a += count_identifiers(left);
                }
            }
            "augmented_assignment" | "named_expression" => self.a += 1,
            "for_statement" | "for_in_clause" => {
                self.c += 1;
                if let Some(left) = n.child_by_field_name("left") {
                    self.a += count_identifiers(left);
                }
            }
            k => {
                if B_KINDS.contains(&k) {
                    self.b += 1;
                }
                if C_KINDS.contains(&k) {
                    self.c += 1;
                }
            }
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }
}

/// Plain identifier targets written by an assignment/loop head; tuple and
/// list targets expand per bound name. Attribute (`obj.attr =`) and
/// subscript (`obj[k] =`) targets reference their operands rather than
/// binding variables, so they contribute nothing.
fn count_identifiers(n: Node) -> u32 {
    match n.kind() {
        "identifier" => 1,
        "attribute" | "subscript" | "call" => 0,
        _ => {
            let mut sum = 0;
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                sum += count_identifiers(child);
            }
            sum
        }
    }
}

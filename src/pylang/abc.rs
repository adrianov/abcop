//! AbcSize over Python trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::PyFile;
use crate::abc::{AbcOffense, offense_at};

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
        if let Some(o) = unit_score(unit, name) {
            offenses.push(o);
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

/// Tally one named unit into its finished offense report.
fn unit_score(unit: Node, name: &str) -> Option<AbcOffense> {
    let t = tally_body(unit)?;
    Some(offense_at(unit, name, t.a, t.b, t.c))
}

/// Walk a unit's `body` subtree through the accumulator.
fn tally_body(unit: Node) -> Option<Tally> {
    let mut t = Tally::default();
    t.walk(unit.child_by_field_name("body")?);
    Some(t)
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
            "assignment" => self.left_targets(n),
            "augmented_assignment" | "named_expression" => self.a += 1,
            "for_statement" | "for_in_clause" => self.loop_head(n),
            k => self.kind_tally(k),
        }
        self.descend(n);
    }

    /// Assignment and loop heads share the left side: every plain
    /// identifier target written there contributes one A.
    fn left_targets(&mut self, n: Node) {
        if let Some(left) = n.child_by_field_name("left") {
            self.a += count_identifiers(left);
        }
    }

    /// Loop heads contribute one C condition on top of their targets.
    fn loop_head(&mut self, n: Node) {
        self.c += 1;
        self.left_targets(n);
    }

    /// Every other kind buckets into B/C according to the kind tables.
    fn kind_tally(&mut self, k: &str) {
        if B_KINDS.contains(&k) {
            self.b += 1;
        }
        if C_KINDS.contains(&k) {
            self.c += 1;
        }
    }

    fn descend(&mut self, n: Node) {
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

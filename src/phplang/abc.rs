//! AbcSize over PHP trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::PhpFile;
use crate::abc::{AbcOffense, fmt_vector};

/// Subtrees that belong to another unit. Anonymous and arrow functions
/// roll into the enclosing unit like Ruby blocks / Rust closures.
fn is_boundary(kind: &str) -> bool {
    matches!(kind, "function_definition" | "method_declaration")
}

const UNIT_KINDS: &[&str] = &["function_definition", "method_declaration"];

/// Binary operators counted toward C; the rest (arithmetic, bitwise,
/// string concat `.`) count toward B.
const C_OPERATORS: &[&str] = &[
    "&&", "||", "and", "or", "xor", "==", "!=", "===", "!==", "<>", "<=>", "<", ">", "<=", ">=",
    "??",
];

pub(crate) fn all_scores(fm: &PhpFile) -> Vec<AbcOffense> {
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

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &'t str)) {
    if UNIT_KINDS.contains(&n.kind()) {
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
struct Tally<'s> {
    src: &'s [u8],
    a: u32,
    b: u32,
    c: u32,
}

impl Tally<'_> {
    fn op_of<'s>(&'s self, n: Node<'s>) -> Option<&'s str> {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
    }

    fn walk(&mut self, n: Node) {
        if is_boundary(n.kind()) {
            return;
        }
        match n.kind() {
            // plain `=`: one A per written identifier target
            "assignment_expression" => {
                if let Some(left) = n.child_by_field_name("left")
                    && left.kind() != "member_access_expression"
                {
                    self.a += count_identifiers(left);
                }
            }
            "augmented_assignment_expression" => self.a += 1,
            // foreach heads bind loop variables exactly like assignments
            "foreach_statement" => {
                self.c += 1;
                if let Some(pair) = child_of_kind(n, "pair") {
                    self.a += count_identifiers(pair);
                }
            }
            "if_statement"
            | "else_if_clause"
            | "while_statement"
            | "do_statement"
            | "for_statement"
            | "case_statement"
            | "default_statement"
            | "match_conditional_expression"
            | "match_default_expression"
            | "catch_clause"
            | "conditional_expression" => self.c += 1,
            "binary_expression" => {
                let is_c = self.op_of(n).is_some_and(|op| C_OPERATORS.contains(&op));
                if is_c {
                    self.c += 1;
                } else {
                    self.b += 1;
                }
            }
            k if k.contains("call_expression")
                || matches!(
                    k,
                    "object_creation_expression"
                        | "include_expression"
                        | "include_once_expression"
                        | "require_expression"
                        | "require_once_expression"
                        | "unary_op_expression"
                ) =>
            {
                self.b += 1
            }
            _ => {}
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }
}

fn child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).find(|ch| ch.kind() == kind)
}

fn count_identifiers(n: Node) -> u32 {
    match n.kind() {
        "variable_name" | "identifier" => 1,
        // reference targets bind nothing
        "member_access_expression"
        | "nullsafe_member_access_expression"
        | "subscript_expression"
        | "function_call_expression" => 0,
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

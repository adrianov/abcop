//! AbcSize over Solidity trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::SolFile;
use crate::abc::{AbcOffense, fmt_vector};

const UNIT_KINDS: &[&str] = &[
    "function_definition",
    "constructor_definition",
    "modifier_definition",
];

fn is_boundary(kind: &str) -> bool {
    UNIT_KINDS.contains(&kind)
}

/// Binary operators counted toward C; arithmetic, bitwise and shifts
/// count toward B.
const C_OPERATORS: &[&str] = &["&&", "||", "==", "!=", "<", ">", "<=", ">="];

pub(crate) fn all_scores(fm: &SolFile) -> Vec<AbcOffense> {
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
    if is_boundary(n.kind()) {
        let name = match n.child_by_field_name("name") {
            Some(name) => Some(name),
            // constructors are anonymous: report them under the
            // enclosing contract's name
            None => {
                let mut anc = n.parent();
                let mut found = None;
                while let Some(a) = anc {
                    if a.kind() == "contract_declaration" {
                        found = a.child_by_field_name("name");
                        break;
                    }
                    anc = a.parent();
                }
                found
            }
        };
        if let Some(name) = name {
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
    fn op_of(&self, n: Node) -> Option<&str> {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
    }

    fn anon_op(&self, n: Node) -> Option<&str> {
        let mut c = n.walk();
        n.children(&mut c)
            .find(|ch| !ch.is_named())
            .and_then(|ch| ch.utf8_text(self.src).ok())
    }

    fn walk(&mut self, n: Node) {
        if is_boundary(n.kind()) {
            return;
        }
        match n.kind() {
            // one A per declared local (tuple heads expand naturally --
            // each element is its own variable_declaration)
            "variable_declaration" => self.a += 1,
            "assignment_expression" => {
                // only plain-variable targets count; state mappings and
                // member writes contribute nothing here (their operands
                // are walked separately)
                if let Some(left) = n.child_by_field_name("left") {
                    self.a += count_plain_identifiers(left);
                }
            }
            "augmented_assignment_expression" | "update_expression" => self.a += 1,
            "if_statement"
            | "for_statement"
            | "while_statement"
            | "do_while_statement"
            | "catch_clause"
            | "conditional_expression" => self.c += 1,
            "binary_expression" => {
                let is_c = self
                    .op_of(n)
                    .or_else(|| self.anon_op(n))
                    .is_some_and(|op| C_OPERATORS.contains(&op));
                if is_c {
                    self.c += 1;
                } else {
                    self.b += 1;
                }
            }
            k if k.contains("call_expression")
                || k == "new_expression"
                || k == "emit_statement"
                || k.ends_with("unary_op_expression")
                || k == "unary_expression" =>
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

/// Identifier targets written by an assignment head; member/array/call
/// targets reference their operands instead and contribute zero.
fn count_plain_identifiers(n: Node) -> u32 {
    match n.kind() {
        "identifier" => 1,
        "member_expression" | "array_access" | "call_expression" => 0,
        _ => {
            let mut sum = 0;
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                sum += count_plain_identifiers(child);
            }
            sum
        }
    }
}

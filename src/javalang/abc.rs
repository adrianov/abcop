//! AbcSize over Java trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::JavaFile;
use crate::abc::{AbcOffense, fmt_vector};

/// Subtrees that belong to another unit; anonymous class bodies roll up
/// into the enclosing unit, whose nested methods still score themselves.
fn is_boundary(kind: &str) -> bool {
    matches!(kind, "method_declaration" | "constructor_declaration")
}

const UNIT_KINDS: &[&str] = &["method_declaration", "constructor_declaration"];

/// Binary operators counted toward C; arithmetic, bitwise and shifts
/// count toward B. `instanceof` is a type test (branch).
const C_OPERATORS: &[&str] = &["&&", "||", "==", "!=", "<", ">", "<=", ">=", "instanceof"];

pub(crate) fn all_scores(fm: &JavaFile) -> Vec<AbcOffense> {
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
    fn op_is_c<'n>(&self, n: Node<'n>) -> bool {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
            .is_some_and(|op| C_OPERATORS.contains(&op))
    }

    fn walk(&mut self, n: Node) {
        if is_boundary(n.kind()) {
            return;
        }
        match n.kind() {
            // one A per declared local
            "variable_declarator" => self.a += 1,
            "assignment_expression" => {
                // plain `=` counts per target; compound reads + rewrites
                let plain = n
                    .child_by_field_name("operator")
                    .and_then(|o| o.utf8_text(self.src).ok())
                    == Some("=");
                if plain {
                    if let Some(left) = n.child_by_field_name("left")
                        && left.kind() == "identifier"
                    {
                        self.a += 1;
                    }
                } else {
                    self.a += 1;
                }
            }
            // enhanced-for heads bind loop variables like declarations
            "enhanced_for_statement" => {
                self.c += 1;
                if let Some(name) = n.child_by_field_name("name") {
                    self.a += u32::from(name.kind() == "identifier");
                }
            }
            // i++ / --i rewrite a variable, exactly like Go's inc/dec
            "update_expression" => self.a += 1,
            "if_statement" | "for_statement" | "while_statement" | "do_statement"
            | "switch_label" | "catch_clause" => self.c += 1,
            "ternary_expression" => self.c += 1,
            "binary_expression" => {
                if self.op_is_c(n) {
                    self.c += 1;
                } else {
                    self.b += 1;
                }
            }
            k if k.contains("invocation")
                || k == "object_creation_expression"
                || k == "explicit_constructor_invocation"
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

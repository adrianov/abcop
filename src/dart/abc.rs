//! AbcSize over Dart trees: unit discovery plus the A/B/C tally.
//!
//! Units are top-level `function_declaration`s plus class-level members
//! (`method_declaration` covering plain methods, getters/setters and all
//! constructor flavors, and the file-level getter/setter declarations).
//! Anonymous function-likes (`function_expression`, local functions) roll
//! into the enclosing unit, mirroring Ruby blocks / Rust closures.

use tree_sitter::Node;

use super::DartFile;
use super::names::unit_name;
use super::scope::bare_target;
use crate::abc::{AbcOffense, fmt_vector};

/// Subtrees that belong to another unit; lambdas and local functions
/// roll into the enclosing unit.
fn is_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration" | "method_declaration" | "getter_declaration" | "setter_declaration"
    )
}

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

#[derive(Default)]
struct Tally<'s> {
    src: &'s [u8],
    a: u32,
    b: u32,
    c: u32,
}

impl Tally<'_> {
    /// Operator token of assignment/unary nodes.
    fn op_of(&self, n: Node<'_>) -> Option<&str> {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
    }

    /// Anonymous trailing token of postfix/prefix update forms.
    fn anon_op(&self, n: Node<'_>) -> Option<&str> {
        let mut c = n.walk();
        n.children(&mut c)
            .filter(|ch| !ch.is_named())
            .find_map(|ch| ch.utf8_text(self.src).ok())
    }

    fn walk(&mut self, n: Node) {
        // visit_units anchors the outermost unit; a named unit can never
        // nest inside another Dart unit, so this stays a pure safety net
        if is_boundary(n.kind()) {
            return;
        }
        match n.kind() {
            // one A per declared local
            "initialized_variable_definition" => self.a += 1,
            "pattern_variable_declaration" => self.a += pattern_target_count(n),
            "for_statement" => {
                self.c += 1;
                // for-in head declares its element variable
                self.a += u32::from(n.child_by_field_name("name").is_some());
            }
            "assignment_expression" => {
                let plain = self.op_of(n) == Some("=");
                // plain writes to bare identifiers are the A payload;
                // compound operators always count, field writes with
                // plain `=` do not (mirrors the C# backend)
                let bare = n
                    .child_by_field_name("left")
                    .and_then(bare_target)
                    .is_some();
                if bare || !plain {
                    self.a += 1;
                }
            }
            k if k.ends_with("_expression") && matches!(self.anon_op(n), Some("++" | "--")) => {
                self.a += 1;
            }
            "postfix_expression" => self.a += 1,
            "unary_expression" => self.b += 1,
            "call_expression"
            | "new_expression"
            | "const_object_expression"
            | "constructor_invocation"
            | "cascade_call_expression" => self.b += 1,
            "additive_expression"
            | "multiplicative_expression"
            | "bitwise_and_expression"
            | "bitwise_or_expression"
            | "bitwise_xor_expression"
            | "shift_expression" => self.b += 1,
            "if_statement"
            | "while_statement"
            | "do_statement"
            | "conditional_expression"
            | "logical_and_expression"
            | "logical_or_expression"
            | "equality_expression"
            | "relational_expression"
            | "if_null_expression"
            | "type_test"
            | "type_test_expression"
            | "type_cast"
            | "type_cast_expression"
            | "switch_statement_case"
            | "switch_statement_default"
            | "switch_expression_case"
            | "catch_clause" => self.c += 1,
            // cascade section carrying an instance-field write
            "cascade_section" if matches!(self.anon_op(n), Some("=")) => self.a += 1,
            _ => {}
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }
}

fn pattern_target_count(n: Node) -> u32 {
    fn count(n: Node, out: &mut u32) {
        if n.kind() == "variable_pattern" {
            *out += 1;
        }
        let mut c = n.walk();
        for ch in n.children(&mut c) {
            count(ch, out);
        }
    }
    let mut out = 0;
    count(n, &mut out);
    out
}

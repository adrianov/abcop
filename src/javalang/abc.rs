//! AbcSize over Java trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::JavaFile;
use crate::abc::{AbcOffense, offense_at};

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
        score_unit(unit, name, fm.src, &mut offenses);
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &'t str)) {
    if UNIT_KINDS.contains(&n.kind())
        && let Some(name) = unit_name(n, src)
    {
        f(n, name);
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(child, src, f);
    }
}

/// The declared identifier of a unit node, if its text decodes.
fn unit_name<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    n.child_by_field_name("name")
        .and_then(|name| name.utf8_text(src).ok())
}

/// Tally one unit subtree and record its offense entry.
fn score_unit(unit: Node, name: &str, src: &[u8], out: &mut Vec<AbcOffense>) {
    let Some(body) = unit.child_by_field_name("body") else {
        return;
    };
    let (a, b, c) = Tally {
        src,
        ..Default::default()
    }
    .over(body);
    out.push(offense_at(unit, name, a, b, c));
}

#[derive(Default)]
struct Tally<'s> {
    src: &'s [u8],
    a: u32,
    b: u32,
    c: u32,
}

impl Tally<'_> {
    /// Score a whole unit body, consuming the tally.
    fn over(mut self, body: Node) -> (u32, u32, u32) {
        self.walk(body);
        (self.a, self.b, self.c)
    }

    /// The node's textual operator, or "" when there is none.
    fn op_text(&self, n: Node) -> &str {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
            .unwrap_or("")
    }

    fn op_is_c(&self, n: Node) -> bool {
        C_OPERATORS.contains(&self.op_text(n))
    }

    fn walk(&mut self, n: Node) {
        if is_boundary(n.kind()) {
            return;
        }
        self.tally(n);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }

    /// Attribute one node to its A/B/C slot by kind.
    fn tally(&mut self, n: Node) {
        match n.kind() {
            // one A per declared local and per i++ / --i rewrite,
            // exactly like Go's inc/dec
            "variable_declarator" | "update_expression" => self.a += 1,
            "assignment_expression" => self.assignment(n),
            // enhanced-for heads bind loop variables like declarations
            "enhanced_for_statement" => self.enhanced_for(n),
            "binary_expression" => self.binary(n),
            // selection and iteration heads, switch labels, catch clauses
            "if_statement" | "for_statement" | "while_statement" | "do_statement"
            | "switch_label" | "catch_clause" | "ternary_expression" => self.c += 1,
            k if is_b_op(k) => self.b += 1,
            _ => {}
        }
    }

    /// Plain `=` counts one A per simple identifier target; compound
    /// operators rewrite-and-read regardless of the target shape.
    fn assignment(&mut self, n: Node) {
        let plain = self.op_text(n) == "=";
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

    fn enhanced_for(&mut self, n: Node) {
        self.c += 1;
        if let Some(name) = n.child_by_field_name("name") {
            self.a += u32::from(name.kind() == "identifier");
        }
    }

    /// Comparisons, logical operators and `instanceof` are control flow
    /// (C); every other binary operator is an operation (B).
    fn binary(&mut self, n: Node) {
        if self.op_is_c(n) {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }
}

/// Kinds counted toward B: anything invocation-shaped, object creation,
/// explicit constructor invocations and unary operators.
fn is_b_op(kind: &str) -> bool {
    kind.contains("invocation")
        || matches!(
            kind,
            "explicit_constructor_invocation" | "object_creation_expression" | "unary_expression"
        )
}

//! AbcSize over Solidity trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::SolFile;
use crate::abc::{AbcOffense, offense_at};

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
        if let Some(o) = unit_offense(fm, unit, name) {
            offenses.push(o);
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

/// Score one unit body and build its offense record; units without a
/// body yield nothing.
fn unit_offense(fm: &SolFile, unit: Node, name: &str) -> Option<AbcOffense> {
    let body = unit.child_by_field_name("body")?;
    let mut t = Tally {
        src: fm.src,
        ..Default::default()
    };
    t.walk(body);
    Some(offense_at(unit, name, t.a, t.b, t.c))
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &'t str)) {
    if is_boundary(n.kind()) {
        if let Some(text) = unit_name(n, src) {
            f(n, text);
        }
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(child, src, f);
    }
}

/// Resolve a unit's display name; constructors are anonymous and
/// report under the enclosing contract's name.
fn unit_name<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    let name = match n.child_by_field_name("name") {
        Some(name) => name,
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
            found?
        }
    };
    name.utf8_text(src).ok()
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
        self.tally_node(n);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }

    fn tally_node(&mut self, n: Node) {
        match n.kind() {
            // one A per declared local (tuple heads expand naturally --
            // each element is its own variable_declaration)
            "variable_declaration" => self.a += 1,
            "assignment_expression" => self.tally_assignment(n),
            "augmented_assignment_expression" | "update_expression" => self.a += 1,
            "if_statement"
            | "for_statement"
            | "while_statement"
            | "do_while_statement"
            | "catch_clause"
            | "conditional_expression" => self.c += 1,
            "binary_expression" => self.tally_binary(n),
            k if Self::counts_as_b(k) => self.b += 1,
            _ => {}
        }
    }

    /// Only plain-variable targets count; state mappings and member
    /// writes contribute nothing here (their operands are walked
    /// separately).
    fn tally_assignment(&mut self, n: Node) {
        if let Some(left) = n.child_by_field_name("left") {
            self.a += count_plain_identifiers(left);
        }
    }

    /// Binary operators split between B and C depending on the
    /// operator.
    fn tally_binary(&mut self, n: Node) {
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

    /// Calls, constructor calls, emits and unary operations all count a B.
    fn counts_as_b(k: &str) -> bool {
        k.contains("call_expression")
            || k == "new_expression"
            || k == "emit_statement"
            || k.ends_with("unary_op_expression")
            || k == "unary_expression"
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

//! AbcSize over PHP trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::PhpFile;
use crate::abc::{AbcOffense, offense_at};

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
        if let Some(offense) = score_unit(unit, name, fm.src) {
            offenses.push(offense);
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

/// Tally one unit's body; a unit without a body yields no offense.
fn score_unit(unit: Node, name: &str, src: &[u8]) -> Option<AbcOffense> {
    let body = unit.child_by_field_name("body")?;
    let mut t = Tally {
        src,
        ..Default::default()
    };
    t.walk(body);
    Some(offense_at(unit, name, t.a, t.b, t.c))
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
            "assignment_expression" => self.tally_assignment(n),
            "augmented_assignment_expression" => self.a += 1,
            "foreach_statement" => self.tally_foreach(n),
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
            "binary_expression" => self.tally_binary(n),
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

    /// plain `=`: one A per written identifier target
    fn tally_assignment(&mut self, n: Node) {
        if let Some(left) = n.child_by_field_name("left")
            && left.kind() != "member_access_expression"
        {
            self.a += count_identifiers(left);
        }
    }

    /// foreach heads bind loop variables exactly like assignments
    fn tally_foreach(&mut self, n: Node) {
        self.c += 1;
        if let Some(pair) = child_of_kind(n, "pair") {
            self.a += count_identifiers(pair);
        }
    }

    /// comparison/logical operators count toward C; the rest toward B
    fn tally_binary(&mut self, n: Node) {
        if self.op_of(n).is_some_and(|op| C_OPERATORS.contains(&op)) {
            self.c += 1;
        } else {
            self.b += 1;
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

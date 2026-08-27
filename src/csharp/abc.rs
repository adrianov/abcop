//! AbcSize over C# trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::CSharpFile;
use crate::abc::{AbcOffense, offense_at};

/// Subtrees that belong to another unit; lambdas and local functions
/// roll into the enclosing unit.
fn is_boundary(kind: &str) -> bool {
    matches!(kind, "method_declaration" | "constructor_declaration")
}

const UNIT_KINDS: &[&str] = &["method_declaration", "constructor_declaration"];

/// Binary operators counted toward C; arithmetic, bitwise, shifts and
/// null-coalescing count toward B... `??` picks a fallback value, which
/// is control logic the same way `?:` is, so it sits with C here.
const C_OPERATORS: &[&str] = &["&&", "||", "==", "!=", "<", ">", "<=", ">=", "is"];

pub(crate) fn all_scores(fm: &CSharpFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm.tree.root_node(), fm.src, &mut |unit, name| {
        push_unit(&mut offenses, fm.src, unit, name);
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

/// Tally one unit subtree and record its offense entry.
fn push_unit(out: &mut Vec<AbcOffense>, src: &[u8], unit: Node<'_>, name: &str) {
    let Some(body) = unit.child_by_field_name("body") else {
        return;
    };
    let mut t = Tally {
        src,
        ..Default::default()
    };
    t.walk(body);
    out.push(offense_at(unit, name, t.a, t.b, t.c));
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &'t str)) {
    if UNIT_KINDS.contains(&n.kind()) {
        if let Some(name) = unit_name(n, src) {
            f(n, name);
        }
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(child, src, f);
    }
}

/// The declared identifier of a unit node, if its text decodes.
fn unit_name<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    n.child_by_field_name("name")?.utf8_text(src).ok()
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
        n.child_by_field_name("operator") // unary/binary/assignment
            .or_else(|| n.child_by_field_name("update_operator"))
            .and_then(|o| o.utf8_text(self.src).ok())
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

    /// One node's contribution to A/B/C.
    fn tally(&mut self, n: Node) {
        match n.kind() {
            // one A per declared local
            "variable_declarator" => self.a += 1,
            "assignment_expression" => self.tally_assignment(n),
            // ++/-- rewrite a variable exactly like Go's inc/dec; other
            // unaries are ordinary value computation.
            k if k.ends_with("unary_expression") => self.tally_unary(n),
            "foreach_statement" => self.tally_foreach(n),
            "if_statement"
            | "for_statement"
            | "while_statement"
            | "do_statement"
            | "switch_section"
            | "catch_clause"
            | "conditional_expression" => self.c += 1,
            "binary_expression" => self.tally_binary(n),
            k if k.contains("invocation_expression") || k == "object_creation_expression" => {
                self.b += 1
            }
            _ => {}
        }
    }

    /// Plain `=` into a bare identifier writes one variable; every
    /// other shape (compound operator, reference target) still counts a
    /// single rewrite.
    fn tally_assignment(&mut self, n: Node) {
        let plain = self.op_of(n) == Some("=");
        if plain
            && n.child_by_field_name("left")
                .is_some_and(|l| l.kind() == "identifier")
        {
            self.a += 1;
        } else if !plain {
            self.a += 1;
        }
    }

    fn tally_unary(&mut self, n: Node) {
        if matches!(self.op_of(n), Some("++" | "--")) {
            self.a += 1;
        } else {
            self.b += 1;
        }
    }

    /// The foreach head branches once and binds its loop variable.
    fn tally_foreach(&mut self, n: Node) {
        self.c += 1;
        if let Some(left) = n.child_by_field_name("left") {
            self.a += u32::from(left.kind() == "identifier");
        }
    }

    /// `&&`/`||`/comparisons branch; remaining binaries just compute.
    fn tally_binary(&mut self, n: Node) {
        if self.op_of(n).is_some_and(|op| C_OPERATORS.contains(&op)) {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }
}

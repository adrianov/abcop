//! AbcSize over Go trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::GoFile;
use crate::abc::{AbcOffense, offense_at};

fn node_text<'s>(src: &'s [u8], n: Node<'s>) -> &'s str {
    n.utf8_text(src).unwrap_or("")
}

/// Subtrees that belong to another unit. Function literals are NOT
/// boundaries: their bodies roll into the enclosing unit like Ruby blocks.
fn is_boundary(kind: &str) -> bool {
    matches!(kind, "function_declaration" | "method_declaration")
}

/// Binary operators counted toward C; every other binary operator
/// (arithmetic, bitwise, shifts) counts toward B, mirroring the Rust
/// backend's condition/operator split.
const C_OPERATORS: &[&str] = &["&&", "||", "==", "!=", "<", ">", "<=", ">="];

/// Assignment operators that rewrite without reading (`=`); anything else
/// (`+=`, `<<=`, ...) reads the previous value too but still counts one A,
/// consistent with other backends' compound assignments.
const PLAIN_ASSIGN_OPS: &[&str] = &["="];

/// Node kinds that name a scoreable unit.
const UNIT_KINDS: &[&str] = &["function_declaration", "method_declaration"];

pub(crate) fn all_scores(fm: &GoFile) -> Vec<AbcOffense> {
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

/// Find every named function/method declaration at any depth, including
/// inside other units' bodies.
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
    /// Operator token of a statement/expression: either a named field or
    /// the first anonymous child (Go's assignment operators are anonymous).
    fn op_of<'s>(src: &'s [u8], n: Node<'s>) -> Option<&'s str> {
        if let Some(op) = n.child_by_field_name("operator") {
            return Some(node_text(src, op));
        }
        let mut c = n.walk();
        n.children(&mut c)
            .find(|ch| !ch.is_named())
            .map(|ch| node_text(src, ch))
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
            // `x := ...`, `x = ...` and range heads: one A per written
            // identifier target
            "short_var_declaration" | "range_clause" => {
                self.a += count_identifiers(n.child_by_field_name("left"));
            }
            "assignment_statement" => self.tally_assignment(n),
            "inc_statement" | "dec_statement" => self.a += 1,
            // Declared names are the identifier children before the `=`;
            // everything after it is the value expression.
            "var_spec" => self.tally_var_spec(n),
            "binary_expression" => self.tally_binary(n),
            "if_statement" | "for_statement" => self.c += 1,
            k if k.ends_with("_case") => self.c += 1,
            "call_expression" | "unary_expression" => self.b += 1,
            _ => {}
        }
    }

    /// Plain `=`/`:=` writes each target once; every other operator is
    /// a single rewrite regardless of target count.
    fn tally_assignment(&mut self, n: Node) {
        if Self::op_of(self.src, n).is_some_and(|op| PLAIN_ASSIGN_OPS.contains(&op)) {
            self.a += count_identifiers(n.child_by_field_name("left"));
        } else {
            self.a += 1;
        }
    }

    fn tally_var_spec(&mut self, n: Node) {
        let mut c = n.walk();
        for child in n.children(&mut c) {
            if !child.is_named() && node_text(self.src, child) == "=" {
                break;
            }
            if child.is_named() && child.kind() == "identifier" {
                self.a += 1;
            }
        }
    }

    /// `&&`/`||`/comparisons branch; remaining binaries just compute.
    fn tally_binary(&mut self, n: Node) {
        if Self::op_of(self.src, n).is_some_and(|op| C_OPERATORS.contains(&op)) {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }
}

/// Identifier targets on an assignment head. Expression-list elements
/// that are references -- selectors (`t.n =`), index expressions
/// (`m[k] =`), calls -- bind no variables and contribute zero.
fn count_identifiers(left: Option<Node>) -> u32 {
    let Some(left) = left else { return 0 };
    fn rec(n: Node) -> u32 {
        match n.kind() {
            "identifier" => 1,
            "selector_expression"
            | "index_expression"
            | "call_expression"
            | "composite_literal" => 0,
            _ => {
                let mut sum = 0;
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    sum += rec(child);
                }
                sum
            }
        }
    }
    rec(left)
}

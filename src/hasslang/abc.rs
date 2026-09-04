//! AbcSize over Haskell trees: unit discovery plus the A/B/C tally.

use tree_sitter::Node;

use super::HsFile;
use super::nodes::{arrow_rhs, bind_name, each_body_child, has_match, is_unit};
use super::patterns::{ignored_name, pattern_a_count};
use crate::abc::{AbcOffense, offense_at};

/// Binary operators counted toward C; arithmetic and the rest count
/// toward B.
const C_OPERATORS: &[&str] = &["&&", "||", "==", "/=", "<", ">", "<=", ">="];

pub(crate) fn all_scores(fm: &HsFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm.tree.root_node(), fm.src, &mut |unit, name| {
        offenses.push(unit_offense(fm, unit, name));
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

fn unit_offense(fm: &HsFile, unit: Node, name: &str) -> AbcOffense {
    let mut t = Tally {
        src: fm.src,
        ..Default::default()
    };
    // Score equation RHS / guards and the where-clause; unit parameters
    // stay outside the tally (protocol, like method params elsewhere).
    each_body_child(unit, |child| t.walk(child));
    offense_at(unit, name, t.a, t.b, t.c)
}

fn visit_units<'t>(n: Node<'t>, src: &'t [u8], f: &mut impl FnMut(Node<'t>, &str)) {
    if is_unit(n) {
        f(n, &unit_name(n, src));
    }
    for child in n.children(&mut n.walk()) {
        visit_units(child, src, f);
    }
}

fn unit_name(n: Node<'_>, src: &[u8]) -> String {
    bind_name(n)
        .and_then(|c| c.utf8_text(src).ok())
        .unwrap_or("<fn>")
        .to_string()
}

#[derive(Default)]
struct Tally<'s> {
    src: &'s [u8],
    a: u32,
    b: u32,
    c: u32,
}

impl Tally<'_> {
    fn walk(&mut self, n: Node) {
        // Nested units are scored on their own visit.
        if is_unit(n) {
            return;
        }
        match n.kind() {
            "bind" => self.walk_bind(n),
            "generator" | "pattern_guard" => self.walk_arrow_bind(n),
            "alternative" => self.walk_alternative(n),
            "lambda" => self.walk_lambda(n),
            _ => {
                self.tally_leaf(n);
                for child in n.children(&mut n.walk()) {
                    self.walk(child);
                }
            }
        }
    }

    /// Named local bind or do-bind: count A, walk expressions only so
    /// pattern constructors never look like calls.
    fn walk_bind(&mut self, n: Node) {
        if has_match(n) {
            self.count_named_a(n);
            each_body_child(n, |child| self.walk(child));
            return;
        }
        self.walk_arrow_bind(n);
    }

    fn walk_arrow_bind(&mut self, n: Node) {
        self.count_arrow_a(n);
        if n.kind() == "pattern_guard" {
            self.c += 1;
        }
        if let Some(expr) = arrow_rhs(n) {
            self.walk(expr);
        }
    }

    fn walk_alternative(&mut self, n: Node) {
        self.c += 1;
        self.count_pat_field(n);
        each_body_child(n, |child| self.walk(child));
    }

    fn walk_lambda(&mut self, n: Node) {
        if let Some(p) = n.child_by_field_name("patterns") {
            self.a += pattern_a_count(p, self.src);
        }
        if let Some(expr) = n.child_by_field_name("expression") {
            self.walk(expr);
        }
    }

    fn count_named_a(&mut self, n: Node) {
        if let Some(name) = bind_name(n) {
            if !ignored_name(name, self.src) {
                self.a += 1;
            }
        }
    }

    fn count_arrow_a(&mut self, n: Node) {
        if let Some(p) = n.child_by_field_name("pattern") {
            self.a += pattern_a_count(p, self.src);
        } else {
            self.count_named_a(n);
        }
    }

    fn count_pat_field(&mut self, n: Node) {
        let p = n
            .child_by_field_name("pattern")
            .or_else(|| n.child_by_field_name("patterns"));
        if let Some(p) = p {
            self.a += pattern_a_count(p, self.src);
        }
    }

    fn tally_leaf(&mut self, n: Node) {
        match n.kind() {
            "conditional" | "boolean" => self.c += 1,
            "match" if n.parent().is_some_and(|p| p.kind() == "multi_way_if") => {
                self.c += 1;
            }
            "apply" | "negation" => self.b += 1,
            "infix" => self.tally_infix(n),
            _ => {}
        }
    }

    fn tally_infix(&mut self, n: Node) {
        if infix_op(n, self.src).is_some_and(|op| C_OPERATORS.contains(&op)) {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }
}

fn infix_op<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    n.children(&mut n.walk())
        .find(|c| c.kind() == "operator")
        .and_then(|c| c.utf8_text(src).ok())
}

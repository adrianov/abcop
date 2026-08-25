
//! C-family language backend: JavaScript, TypeScript, C, C++, Objective-C
//! and Swift AbcSize over the shared tree-sitter tree.
//!
//! Metric spec (mirrors the Rust backend's semantics):
//! - Units are *named* declarations: JS/TS `function_declaration`,
//!   `generator_function_declaration`, `method_definition`, and arrow /
//!   function expressions bound to a name by their enclosing
//!   variable_declarator, assignment, object pair or class field; C/C++
//!   `function_definition` and ObjC `function_definition` /
//!   `method_definition`; Swift `function_declaration` / `init_declaration`.
//! - Anonymous function-likes are NOT units: their contents roll into the
//!   enclosing unit (mirrors Ruby blocks / Rust closures).
//! - A unit's score walks its whole body but never descends into another
//!   unit root -- those carry their own offense, so nothing double-counts.
//! - A: plain and compound assignments, `++`/`--` updates,
//!   variable_declarator/init_declarator carrying an initializer.
//! - B: call/new/message expressions, yield, throw; unary operators except
//!   pointer `*`/`&`; non-condition binary operators (arithmetic, shift,
//!   bitwise); Swift arithmetic expressions and prefixes.
//! - C: if/ternary/guard, loops, switch arms, catch clauses, comparisons,
//!   logical operators (`&& || ??`), Swift conjunction/disjunction/
//!   nil-coalescing/comparison expressions.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::abc::{fmt_vector, AbcOffense};
use crate::paths::Lang;

const COMPARISONS: &[&str] =
    &["==", "===", "!=", "!==", "<", ">", "<=", ">=", "<=>"];
const LOGICAL: &[&str] = &["&&", "||", "??"];
/// Unary operators counted as branches; pointer `*`/`&` are memory access,
/// not computation, and stay uncounted.
const UNARY_B: &[&str] = &["!", "~", "-", "+"];

const JS_UNITS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "method_definition",
];
const JS_ANON: &[&str] = &["arrow_function", "function", "generator_function"];
const JS_ASSIGNS: &[&str] = &[
    "assignment_expression",
    "augmented_assignment_expression",
    "update_expression",
    "variable_declarator",
];
const JS_CALLS: &[&str] = &[
    "call_expression",
    "new_expression",
    "yield_expression",
    "throw_statement",
];
const JS_CONDS: &[&str] = &[
    "if_statement",
    "ternary_expression",
    "while_statement",
    "do_statement",
    "for_statement",
    "for_in_statement",
    "catch_clause",
    "switch_case",
    "switch_default",
];

const C_ASSIGNS: &[&str] =
    &["init_declarator", "assignment_expression", "update_expression"];

struct Spec {
    /// Named declaration kinds: each becomes a scored unit.
    units: &'static [&'static str],
    /// Anonymous function-like kinds that may be name-bound by a parent;
    /// unbound ones roll into the enclosing unit.
    anon: &'static [&'static str],
    assigns: &'static [&'static str],
    calls: &'static [&'static str],
    conds: &'static [&'static str],
    /// Assign kinds counted only when they carry an initializer.
    conditional_assigns: &'static [&'static str],
    /// Kind holding an operator token to classify as B vs C. Empty for
    /// grammars that encode the distinction in the node kind.
    op_binary_kind: &'static str,
    /// Kinds that are always a branch (arithmetic operators in grammars
    /// without a generic binary node).
    op_arith_kinds: &'static [&'static str],
}

fn spec_for(lang: Lang) -> Spec {
    match lang {
        Lang::Js | Lang::Ts | Lang::Tsx => Spec {
            units: JS_UNITS,
            anon: JS_ANON,
            assigns: JS_ASSIGNS,
            calls: JS_CALLS,
            conds: JS_CONDS,
            conditional_assigns: &["variable_declarator"],
            op_binary_kind: "binary_expression",
            op_arith_kinds: &[],
        },
        Lang::C | Lang::Cpp => Spec {
            units: &["function_definition"],
            anon: &[],
            assigns: C_ASSIGNS,
            calls: &["call_expression", "new_expression"],
            conds: &[
                "if_statement",
                "while_statement",
                "do_statement",
                "for_statement",
                "case_statement",
                "catch_clause",
            ],
            conditional_assigns: &[],
            op_binary_kind: "binary_expression",
            op_arith_kinds: &[],
        },
        Lang::ObjC => Spec {
            units: &["function_definition", "method_definition"],
            anon: &[],
            assigns: C_ASSIGNS,
            calls: &["call_expression", "message_expression", "throw_statement"],
            conds: &[
                "if_statement",
                "while_statement",
                "do_statement",
                "for_statement",
                "case_statement",
                "catch_clause",
            ],
            conditional_assigns: &[],
            op_binary_kind: "binary_expression",
            op_arith_kinds: &[],
        },
        Lang::Swift => Spec {
            units: &["function_declaration", "init_declaration"],
            anon: &[],
            assigns: &["assignment"],
            calls: &["call_expression", "throw_keyword", "prefix_expression"],
            conds: &[
                "if_statement",
                "guard_statement",
                "while_statement",
                "repeat_while_statement",
                "for_statement",
                "do_statement",
                "switch_entry",
                "catch_block",
                "ternary_expression",
                "comparison_expression",
                "conjunction_expression",
                "disjunction_expression",
                "nil_coalescing_expression",
            ],
            conditional_assigns: &["property_declaration"],
            // Swift operators parse as infix_expression with an operator
            // token; arithmetic-only kinds are separate nodes.
            op_binary_kind: "infix_expression",
            op_arith_kinds: &[
                "additive_expression",
                "multiplicative_expression",
                "bitwise_operation",
            ],
        },
        _ => unreachable!("clike backend invoked for a non-clike language"),
    }
}

fn node_text<'t>(n: Node<'t>, src: &'t [u8]) -> &'t str {
    std::str::from_utf8(&src[n.start_byte()..n.end_byte()]).unwrap_or("")
}

/// Operator token of a binary/unary node. Grammars disagree on the field
/// name: JS/C use `operator`, Swift uses `op`.
fn op_text<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    let op = n
        .child_by_field_name("operator")
        .or_else(|| n.child_by_field_name("op"))?;
    Some(node_text(op, src))
}

/// Display name of a unit root: `name` field, then a declarator chain
/// (C/C++), then the first identifier-shaped child before the body
/// (ObjC selectors), else `(anonymous)`.
fn declared_name(n: Node, src: &[u8]) -> String {
    if let Some(name) = n.child_by_field_name("name") {
        return node_text(name, src).to_string();
    }
    let mut cur = n.child_by_field_name("declarator");
    while let Some(c) = cur {
        match c.kind() {
            "identifier" | "field_identifier" | "property_identifier" => {
                return node_text(c, src).to_string();
            }
            _ => cur = c.child_by_field_name("declarator"),
        }
    }
    // ObjC: the selector parts precede the body; take the first one.
    let mut cursor = n.walk();
    let mut stack: Vec<Node> = n.children(&mut cursor).collect();
    while let Some(c) = stack.pop() {
        if matches!(c.kind(), "body" | "compound_statement") {
            continue;
        }
        if matches!(
            c.kind(),
            "identifier" | "method_identifier" | "field_identifier"
        ) {
            return node_text(c, src).to_string();
        }
        let mut inner = c.walk();
        stack.extend(c.children(&mut inner));
    }
    "(anonymous)".to_string()
}

/// Name bound to an anonymous function-like by its parent, if any: the
/// `const f = () => {}` idiom.
fn anon_bound_name<'t>(n: Node<'t>, src: &'t [u8]) -> Option<String> {
    let p = n.parent()?;
    let (value_field, name_field) = match p.kind() {
        "variable_declarator"
        | "pair"
        | "property_definition"
        | "public_field_definition" => ("value", "name"),
        "assignment_expression" => ("right", "left"),
        _ => return None,
    };
    if p.child_by_field_name(value_field)? != n {
        return None;
    }
    let key = p.child_by_field_name(name_field)?;
    Some(
        node_text(key, src)
            .trim_matches(|c: char| c == '\'' || c == '"')
            .to_string(),
    )
}
fn discover<'t>(
    spec: &Spec,
    n: Node<'t>,
    src: &[u8],
    out: &mut Vec<(Node<'t>, String)>,
    roots: &mut HashSet<usize>,
) {
    let kind = n.kind();
    if spec.units.contains(&kind) {
        out.push((n, declared_name(n, src)));
        roots.insert(n.start_byte());
    } else if spec.anon.contains(&kind)
        && let Some(name) = anon_bound_name(n, src)
    {
        out.push((n, name));
        roots.insert(n.start_byte());
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        discover(spec, child, src, out, roots);
    }
}

struct Tally {
    a: u32,
    b: u32,
    c: u32,
}

impl Tally {
    fn count(&mut self, spec: &Spec, n: Node, src: &[u8]) {
        let k = n.kind();
        let counted_assignment = spec.assigns.contains(&k)
            && (!spec.conditional_assigns.contains(&k)
                || n.child_by_field_name("value").is_some());
        if counted_assignment {
            self.a += 1;
            return;
        }
        if spec.calls.contains(&k) {
            self.b += 1;
            return;
        }
        if spec.conds.contains(&k) {
            self.c += 1;
            return;
        }
        if spec.op_arith_kinds.contains(&k) {
            self.b += 1;
            return;
        }
        if k == spec.op_binary_kind {
            match op_text(n, src) {
                Some(op) if COMPARISONS.contains(&op) => self.c += 1,
                Some(op) if LOGICAL.contains(&op) => self.c += 1,
                Some(_) => self.b += 1,
                None => {}
            }
            return;
        }
        if k == "unary_expression"
            && let Some(op) = op_text(n, src)
            && UNARY_B.contains(&op)
        {
            self.b += 1;
        }
    }

    /// Walk a unit body post-order, skipping subtrees rooted at other units
    /// so no construct counts twice.
    fn walk(&mut self, spec: &Spec, n: Node, src: &[u8], roots: &HashSet<usize>) {
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            if !roots.contains(&child.start_byte()) {
                self.walk(spec, child, src, roots);
            }
        }
        self.count(spec, n, src);
    }
}

pub(crate) fn analyze(
    src: &[u8],
    tree: &Tree,
    lang: Lang,
    max: f64,
) -> Vec<AbcOffense> {
    let spec = spec_for(lang);
    let mut units = Vec::new();
    let mut roots = HashSet::new();
    discover(&spec, tree.root_node(), src, &mut units, &mut roots);

    let mut offenses = Vec::new();
    for (unit, name) in units {
        let Some(body) = unit_body(unit) else {
            continue;
        };
        let mut t = Tally { a: 0, b: 0, c: 0 };
        t.walk(&spec, body, src, &roots);
        let pos = unit.start_position();
        let raw = ((t.a * t.a + t.b * t.b + t.c * t.c) as f64).sqrt();
        offenses.push(AbcOffense {
            line: pos.row + 1,
            end_line: unit.end_position().row + 1,
            column: pos.column,
            name,
            score: (raw * 100.0).round() / 100.0,
            vector: fmt_vector(t.a, t.b, t.c),
        });
    }
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses.retain(|o| o.score > max);
    offenses
}

/// Unit body: the `body` field when the grammar has one (JS/TS/Swift,
/// C/C++ function_definition), else the first compound statement child
/// (ObjC method_definition carries no fields).
fn unit_body<'t>(n: Node<'t>) -> Option<Node<'t>> {
    if let Some(b) = n.child_by_field_name("body") {
        return Some(b);
    }
    let mut cursor = n.walk();
    n.children(&mut cursor)
        .find(|ch| ch.kind() == "compound_statement")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(lang: Lang, code: &str, max: f64) -> Vec<AbcOffense> {
        let tree =
            crate::paths::parse_file_lang(code.as_bytes(), lang).unwrap();
        analyze(code.as_bytes(), &tree, lang, max)
    }

    #[test]
    fn javascript_counts_assignments_calls_and_conditions() {
        let code = "\
function checkout(user, items) {
  let total = 0;
  for (const it of items) {
    total = total + it.price;
  }
  if (user.vip) {
    total = applyDiscount(total);
  }
  log(total);
}
";
        // Force all scores out for exact-vector assertions.
        let off = scores(Lang::Js, code, 0.0);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].name, "checkout");
        // A: total init, total += price, total = applyDiscount(...)
        // B: applyDiscount call, log call, `+` arithmetic operator
        // C: for-of, if
        assert_eq!(off[0].vector, "<3, 3, 2>");
        assert!(scores(Lang::Js, code, 17.0).is_empty());
    }

    #[test]
    fn ts_const_arrow_is_a_named_unit_and_nested_fns_do_not_double_count() {
        let code = "\
const outer = (xs) => {
  function inner(y) {
    return helper(y) + helper(y);
  }
  return inner(1);
};
";
        let off = scores(Lang::Ts, code, 0.0);
        assert_eq!(off.len(), 2, "{off:?}");
        let inner = off.iter().find(|o| o.name == "inner").unwrap();
        let outer = off.iter().find(|o| o.name == "outer").unwrap();
        // Inner owns helper x2 (+ operator); outer only its own call.
        assert_eq!(inner.vector, "<0, 3, 0>");
        assert_eq!(outer.vector, "<0, 1, 0>");
    }

    #[test]
    fn c_counts_loop_body_with_updates_and_comparisons() {
        let code = "\
int sum_upto(int n) {
  int s = 0;
  for (int i = 0; i < n; i++) {
    s += i;
  }
  return s;
}
";
        let off = scores(Lang::C, code, 0.0);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].name, "sum_upto");
        // A: s=0, i=0 init, s+=i, i++ update
        // B: none (comparisons are conditions here)
        // C: for, i<n comparison
        assert_eq!(off[0].vector, "<4, 0, 2>");
    }

    #[test]
    fn objective_c_message_sends_are_branches() {
        let code = "\
@implementation Widget
- (int)pick:(NSArray *)items {
  if ([items count] > 0) {
    return [[self factory] build];
  }
  return 0;
}
@end
";
        let off = scores(Lang::ObjC, code, 0.0);
        assert_eq!(off.len(), 1, "{off:?}");
        // B: [items count], [self factory], [.. build] = 3 sends
        // C: if, > = 2
        assert_eq!(off[0].vector, "<0, 3, 2>");
    }

    #[test]
    fn swift_functions_score_on_the_same_scale() {
        let code = "\
func grade(_ score: Int) -> String {
  if score >= 90 {
    return \"A\"
  } else if score >= 80 {
    return \"B\"
  }
  return \"F\"
}
";
        let off = scores(Lang::Swift, code, 0.0);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].name, "grade");
        // C: two ifs + two >= comparisons
        assert_eq!(off[0].vector, "<0, 0, 4>");
    }

    #[test]
    fn cpp_throw_and_new_are_branches() {
        let code = "\
Widget* make(int n) {
  if (n <= 0) throw std::invalid_argument(\"n\");
  return new Widget(n);
}
";
        let off = scores(Lang::Cpp, code, 0.0);
        assert_eq!(off.len(), 1);
        // B: throw, new
        // C: if, <= comparison
        assert_eq!(off[0].vector, "<0, 2, 2>");
    }
}

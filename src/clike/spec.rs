//! Per-language metric specs for the C-family backend: which tree-sitter
//! kinds count as units, assignments, calls and conditions for JavaScript,
//! TypeScript, C, C++, Objective-C and Swift, plus shared tree-text utils.
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

use tree_sitter::Node;

use crate::paths::Lang;

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

const C_ASSIGNS: &[&str] = &[
    "init_declarator",
    "assignment_expression",
    "update_expression",
];

pub(crate) struct Spec {
    /// Named declaration kinds: each becomes a scored unit.
    pub units: &'static [&'static str],
    /// Anonymous function-like kinds that may be name-bound by a parent;
    /// unbound ones roll into the enclosing unit.
    pub anon: &'static [&'static str],
    pub assigns: &'static [&'static str],
    pub calls: &'static [&'static str],
    pub conds: &'static [&'static str],
    /// Assign kinds counted only when they carry an initializer.
    pub conditional_assigns: &'static [&'static str],
    /// Kind holding an operator token to classify as B vs C. Empty for
    /// grammars that encode the distinction in the node kind.
    pub op_binary_kind: &'static str,
    /// Kinds that are always a branch (arithmetic operators in grammars
    /// without a generic binary node).
    pub op_arith_kinds: &'static [&'static str],
}

pub(crate) fn spec_for(lang: Lang) -> Spec {
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

pub(crate) fn node_text<'t>(n: Node<'t>, src: &'t [u8]) -> &'t str {
    std::str::from_utf8(&src[n.start_byte()..n.end_byte()]).unwrap_or("")
}

/// Operator token of a binary/unary node. Grammars disagree on the field
/// name: JS/C use `operator`, Swift uses `op`.
pub(crate) fn op_text<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    let op = n
        .child_by_field_name("operator")
        .or_else(|| n.child_by_field_name("op"))?;
    Some(node_text(op, src))
}

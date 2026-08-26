//! RHS purity predicates and NeverUsed analysis.

use tree_sitter::Node;

use super::scope::{Entry, RustFile, Write};

/// Conservative RHS purity: literals, constant paths, and compositions of
/// comparisons/logical/arithmetic over those. Calls, macros, `?`, field reads
/// through non-const bases, and local-variable references are all rejected.
pub(super) fn pure(fm: &RustFile, n: Node) -> bool {
    match n.kind() {
        "integer_literal" | "float_literal" | "char_literal" | "string_literal"
        | "raw_string_literal" | "true" | "false" | "unit_type" => true,
        "scoped_identifier" => true, // constants; enforced immutable by rustc
        "reference_expression" | "unary_expression" => children_pure(fm, n),
        "binary_expression" | "tuple_expression" | "array_expression" | "range_expression" => {
            children_pure(fm, n)
        }
        "type_cast_expression" | "field_expression" => child_pure(fm, n),
        _ => false,
    }
}

fn children_pure<'t>(fm: &RustFile, n: Node<'t>) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(fm, ch))
}

/// Purity of the node's `value` field (casts and field reads).
fn child_pure<'t>(fm: &RustFile, n: Node<'t>) -> bool {
    n.child_by_field_name("value")
        .map(|v| pure(fm, v))
        .unwrap_or(false)
}

/// Straight-line execution check up to the nearest function/closure/block
/// boundary (the binding's scope).
pub(super) fn unconditionally_executed(write_node: Node) -> bool {
    const VETO: [&str; 7] = [
        "if_expression",
        "if_let_expression",
        "while_expression",
        "while_let_expression",
        "for_expression",
        "match_arm",
        "match_expression",
    ];
    const OWNERS: [&str; 3] = ["function_item", "closure_expression", "block"];
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if VETO.contains(&n.kind()) {
            return false;
        }
        if OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

/// NeverUsed for Rust sources: bindings with writes but zero reads whose
/// initializer carries no macro invocation.
pub fn never_used_offenses(fm: &RustFile) -> Vec<crate::never_used::NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if never_used_entry(fm, e) {
                out.push(offense_at_first_write(fm, name, e));
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

/// Zero reads overall and no macro-interpolated initializer anywhere.
fn never_used_entry(fm: &RustFile, e: &Entry) -> bool {
    e.reads.is_empty() && !e.writes.is_empty() && e.writes.iter().all(|w| macro_free_rhs(fm, w))
}

/// A write qualifies when its RHS subtree contains no macro invocation.
fn macro_free_rhs(fm: &RustFile, w: &Write) -> bool {
    let Some((_, byte)) = w.rhs else {
        return true;
    };
    fm.tree
        .root_node()
        .descendant_for_byte_range(byte, byte)
        .map(|node| !contains_macro(node))
        .unwrap_or(true)
}

fn offense_at_first_write(
    fm: &RustFile,
    name: &str,
    e: &Entry,
) -> crate::never_used::NeverUsedOffense {
    let first = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
    let (line, column) = fm.line_col(first);
    crate::never_used::NeverUsedOffense {
        line,
        column,
        name: name.to_string(),
    }
}

fn contains_macro(n: Node) -> bool {
    let mut cursor = n.walk();
    n.children(&mut cursor)
        .any(|c| c.kind() == "macro_invocation" || contains_macro(c))
}

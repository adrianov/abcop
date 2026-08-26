//! Rust-language backend: scope model, AbcSize, and used-once analysis.
//!
//! Metric spec (defined to mirror the Ruby port's semantics where a direct
//! analogue exists):
//! - Units: every `function_item` (free fns and impl methods), scored over its
//!   `@body` subtree, post-order. Closures are not separate units; their params
//!   and contents roll into the enclosing function (mirrors Ruby blocks).
//! - A: let bindings (per pattern identifier), `=` and compound assignments,
//!   `for`/`if let`/`while let`/match-arm pattern bindings, closure params,
//!   params of nested functions. Underscore-prefixed names never count.
//! - B: call expressions, macro invocations, `?` try expressions, unary ops,
//!   non-condition binary ops.
//! - C: if / if-let / while / while-let / for, one per match arm (guards come
//!   via normal binary rules), comparisons and `&&`/`||`.
//!   No else bonus: Rust if-else is a value-producing expression.
//! - UsedOnce: single plain `let`, single read, pure RHS, straight-line write,
//!   read after write. Params/pattern-bound/mut-reassigned vars excluded.

mod bindings;
mod builder;
mod entries;
mod format;
mod patterns;
mod pure;
mod scope;
#[cfg(test)]
mod tests;
mod units;
mod usedonce;

use tree_sitter::Node;

use crate::abc::{AbcOffense, fmt_vector};

pub use builder::build;
pub use pure::never_used_offenses;
pub use scope::RustFile;
pub use usedonce::used_once_offenses;

/// Node kinds whose subtrees are type/attribute territory — no variable reads
/// or metric contributions live there.
fn skip_subtree(kind: &str) -> bool {
    if matches!(
        kind,
        "type_arguments"
            | "type_parameters"
            | "where_clause"
            | "trait_bounds"
            | "attribute_item"
            | "scoped_type_identifier"
            | "metavariable"
            | "line_comment"
            | "scoped_identifier"
    ) {
        return true;
    }
    // real types (`reference_type`, `generic_type`, …) — but not casts
    kind.ends_with("_type") && kind != "type_cast_expression"
}

fn visit_units(fm: &RustFile, n: Node, f: &mut impl FnMut(Node, &str)) {
    let is_fn = n.kind() == "function_item";
    if is_fn && let Some(name_node) = n.child_by_field_name("name") {
        f(n, fm.text(name_node));
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(fm, child, f);
    }
}

fn score_unit(fm: &RustFile, unit: Node, name: &str) -> AbcOffense {
    let calc = units::Calc::over(fm);
    if let Some(body) = unit.child_by_field_name("body") {
        let (a, b, c) = calc.score(body);
        return offense_at(unit, name, a, b, c);
    }
    offense_at(unit, name, 0, 0, 0)
}

/// Position an AbcOffense at its unit root with rounded score and vector.
fn offense_at(unit: Node, name: &str, a: u32, b: u32, c: u32) -> AbcOffense {
    let raw = ((a * a + b * b + c * c) as f64).sqrt();
    let pos = unit.start_position();
    AbcOffense {
        line: pos.row + 1,
        end_line: unit.end_position().row + 1,
        column: pos.column,
        name: name.to_string(),
        score: (raw * 100.0).round() / 100.0,
        vector: fmt_vector(a, b, c),
    }
}

pub fn all_scores(fm: &RustFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm, fm.tree.root_node(), &mut |unit, name| {
        if unit.child_by_field_name("body").is_some() {
            offenses.push(score_unit(fm, unit, name));
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

pub fn analyze(fm: &RustFile, max: f64) -> Vec<AbcOffense> {
    all_scores(fm)
        .into_iter()
        .filter(|o| o.score > max)
        .collect()
}

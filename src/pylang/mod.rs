//! Python-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust backend's semantics where a direct
//! analogue exists):
//! - Units: every named `function_definition` (free functions, methods,
//!   nested funcs). Lambdas are NOT units; their contents roll into the
//!   enclosing unit (mirrors Ruby blocks / Rust closures). Nested
//!   function/class definitions never descend into a parent's score --
//!   those carry their own offense.
//! - A: assignments (+1 per written identifier target), augmented
//!   assignments, walrus expressions, `for` and comprehension loop
//!   targets.
//! - B: calls, attribute reads, subscripts, arithmetic/bitwise/unary
//!   operators, f-string interpolations.
//! - C: if / elif / while / for (incl. async forms), comprehension
//!   for/if clauses, ternary conditional expressions, each except
//!   clause, each match case arm, comparisons, `and`/`or`.
//! - UsedOnce: single non-augmented write, pure RHS, straight-line
//!   write position, single read after the write. Parameters, loop and
//!   `with ... as` / `except ... as` bindings, imports and
//!   underscore-prefixed names are excluded.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.
mod abc;
mod vars;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_usage;

use tree_sitter::Tree;

pub struct PyFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<Scope>,
}

impl PyFile<'_> {
    /// Line/column (1-based line, 0-based column) for a byte offset.
    pub(super) fn line_col(&self, byte: usize) -> (usize, usize) {
        let point = self
            .tree
            .root_node()
            .descendant_for_byte_range(byte, byte)
            .map(|n| n.start_position())
            .unwrap_or_default();
        (point.row + 1, point.column)
    }
}

pub fn build(src: &[u8], tree: Tree) -> PyFile<'_> {
    let scopes = vars::collect(tree.root_node(), src);
    PyFile { src, tree, scopes }
}

use vars::Scope;
pub(crate) use vars::{never_used_offenses, used_once_offenses};

pub fn analyze(fm: &PyFile, max: f64) -> Vec<crate::abc::AbcOffense> {
    let mut offenses = abc::all_scores(fm);
    offenses.retain(|o| o.score > max);
    offenses
}

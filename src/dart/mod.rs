//! Dart-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust/C# backends' semantics where a direct
//! analogue exists):
//! - Units: top-level `function_declaration`s plus class-level members --
//!   every `method_declaration` (plain methods, getters/setters,
//!   constructors/factories) and file-level `getter_declaration` /
//!   `setter_declaration`. Closures and local functions roll into the
//!   enclosing unit.
//! - A: local declarators (+1 per declared name), pattern-declared names,
//!   plain assignments to bare identifiers, compound assignments, `++`/`--`,
//!   for-in heads.
//! - B: invocations, constructor invocations, cascade calls, unary
//!   operators (except `++`/`--`), arithmetic/bitwise/shift binary operators.
//! - C: if / for / while / do, switch cases and defaults, catch clauses,
//!   ternaries, comparisons, `&&`/`||`, `??`, `is` tests and `as` casts.
//! - UsedOnce: single plain write, pure RHS, straight-line write, single
//!   read after the write. Parameters, for-in heads and catch bindings are
//!   protocol, never candidates.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.
//!
//! Dart mirrors Swift's closure model: every unit opens a [`Block`]
//! scope -- methods/local functions/closures capture outer bindings -- so
//! nothing hard-severs resolution; root-scope bindings stay unreported
//! (`include_root_scope: false`) because top-level finals may be consumed
//! by other libraries.

mod abc;
mod names;
mod patterns;
mod scope;

#[cfg(test)]
mod tests;
mod usage;

use crate::scope_model::Scope;
use tree_sitter::Tree;

pub use usage::{never_used_offenses, used_once_offenses};

pub struct DartFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<Scope>,
}

impl DartFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> DartFile<'_> {
    let scopes = scope::collect(tree.root_node(), src);
    DartFile { src, tree, scopes }
}

pub fn analyze(fm: &DartFile, max: f64) -> Vec<crate::abc::AbcOffense> {
    let mut offenses = abc::all_scores(fm);
    offenses.retain(|o| o.score > max);
    offenses
}

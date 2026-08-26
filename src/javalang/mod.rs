//! Java-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust backend's semantics where a direct
//! analogue exists):
//! - Units: every `method_declaration` and `constructor_declaration`.
//!   Lambdas and anonymous class bodies roll into the enclosing unit.
//! - A: local variable declarators (+1 per declared name), plain
//!   assignments to variables, augmented assignments, enhanced-for
//!   heads.
//! - B: method invocations, constructor calls (`new`), explicit
//!   constructor invocations, unary operators, arithmetic/bitwise/
//!   shift binary operators.
//! - C: if / for / enhanced-for / while / do, every switch label,
//!   catch clauses, ternaries, comparisons, `&&`/`||`, `instanceof`.
//! - UsedOnce: single plain write, pure RHS, straight-line write,
//!   single read after the write. Parameters, catch/resources
//!   bindings and enhanced-for heads are protocol, never candidates.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.

use crate::scope_model::Scope;

mod abc;
mod scope;
mod usage;

#[cfg(test)]
mod tests;

use tree_sitter::Tree;

pub use usage::{never_used_offenses, used_once_offenses};

pub struct JavaFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<Scope>,
}

impl JavaFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> JavaFile<'_> {
    let scopes = scope::collect(tree.root_node(), src);
    JavaFile { src, tree, scopes }
}

pub fn analyze(fm: &JavaFile, max: f64) -> Vec<crate::abc::AbcOffense> {
    let mut offenses = abc::all_scores(fm);
    offenses.retain(|o| o.score > max);
    offenses
}

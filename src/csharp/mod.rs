//! C#-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust backend's semantics where a direct
//! analogue exists):
//! - Units: every `method_declaration` and `constructor_declaration`.
//!   Lambdas, anonymous methods and local functions roll into the
//!   enclosing unit.
//! - A: local declarators (+1 per declared name), plain assignments,
//!   augmented assignments, foreach heads, `++`/`--`.
//! - B: invocations, object creations, unary operators, arithmetic/
//!   bitwise/shift binary operators.
//! - C: if / for / foreach / while / do, every switch section, catch
//!   clauses, ternaries, comparisons, `&&`/`||`, `is` type tests.
//! - UsedOnce: single plain write, pure RHS, straight-line write, single
//!   read after the write. Parameters, foreach heads and catch bindings
//!   are protocol, never candidates.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.

pub(crate) mod abc;
mod scope;

#[cfg(test)]
mod tests;
mod usage;

use crate::scope_model::Scope;
use tree_sitter::Tree;

pub use usage::{never_used_offenses, used_once_offenses};

pub struct CSharpFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<Scope>,
}

impl CSharpFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> CSharpFile<'_> {
    CSharpFile {
        src,
        scopes: scope::collect(tree.root_node(), src),
        tree,
    }
}

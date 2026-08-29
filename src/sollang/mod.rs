//! Solidity-language backend: scope model, AbcSize, used-once/
//! never-used.
//!
//! Metric spec (mirrors the Rust backend's semantics where a direct
//! analogue exists):
//! - Units: every `function_definition`, `constructor_definition` and
//!   `modifier_definition`.
//! - A: local declarations (+1 per declared name, tuples expand), plain
//!   assignments to visible locals, augmented assignments, `++`/`--`.
//! - B: calls, `new` expressions, emits, arithmetic/bitwise/shift
//!   binary operators, unary operators.
//! - C: if / for / while / do-while, try and each catch clause,
//!   ternaries, comparisons, `&&`/`||`.
//! - UsedOnce: single plain write, pure RHS, straight-line write,
//!   single read after the write. Parameters and tuple heads are
//!   protocol; assignments targeting undeclared names (state
//!   variables) contribute operand reads only -- Solidity has no
//!   undeclared locals either.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.

pub(crate) mod abc;
mod decl;
mod scope;
mod usage;

pub use usage::{never_used_offenses, used_once_offenses};

#[cfg(test)]
mod tests;
use tree_sitter::Tree;

pub struct SolFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<crate::scope_model::Scope>,
}

impl SolFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> SolFile<'_> {
    let scopes = scope::collect(tree.root_node(), src);
    SolFile { src, tree, scopes }
}

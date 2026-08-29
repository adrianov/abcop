//! PHP-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust backend's semantics where a direct
//! analogue exists):
//! - Units: every named `function_definition` and `method_declaration`.
//!   Anonymous and arrow functions roll into the enclosing unit -- they
//!   are NOT boundaries here.
//! - A: assignments (+1 per written identifier target, destructuring
//!   lists expand per name), augmented assignments, foreach heads.
//! - B: calls (plain, method, scoped, nullsafe), `new`, includes,
//!   arithmetic/bitwise/concat binary operators, unary operators.
//! - C: if / elseif / while / do / for / foreach, each switch case and
//!   default, each match arm, catch clauses, ternaries, comparisons,
//!   `&&`/`||`/`and`/`or`/`??`.
//! - UsedOnce: single plain write, pure RHS, straight-line write, single
//!   read after the write. Parameters, foreach heads, `$this`, catch
//!   bindings and underscore-free-only naming rules apply as elsewhere
//!   (PHP names lose their leading `$` internally).
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.

use crate::scope_model::Scope;

pub(crate) mod abc;
mod scope;
mod usage;

#[cfg(test)]
mod tests;

use tree_sitter::Tree;

pub use usage::{never_used_offenses, used_once_offenses};

pub struct PhpFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<Scope>,
}

impl PhpFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> PhpFile<'_> {
    let scopes = scope::collect(tree.root_node(), src);
    PhpFile { src, tree, scopes }
}

//! Zig-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust/Go backends' semantics where a direct
//! analogue exists):
//! - Units: every `function_declaration` with a body, plus
//!   `test_declaration` and `comptime_declaration` blocks. Nested
//!   methods inside `struct`/`union` containers are separate units.
//! - A: `const`/`var` declarations (+1 per declared name), assignment
//!   targets (statement form aliased as `variable_declaration`, plus
//!   `assignment_expression`), and payload bindings (`|x|`).
//! - B: calls (including `@builtin`), arithmetic/bitwise/shift binary
//!   operators, unary operators, `try`.
//! - C: if / for / while (statement and expression forms), each switch
//!   case, `catch`, comparisons, `and`/`or`/`orelse`.
//! - UsedOnce: single plain write, pure RHS, straight-line write, single
//!   read after the write. Parameters and payloads are protocol.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions. Root-scope container bindings stay unreported
//!   (`include_root_scope: false`) because module/struct state may be
//!   consumed elsewhere.

pub(crate) mod abc;
mod decl;
mod scope;
mod usage;

#[cfg(test)]
mod tests;

use tree_sitter::Tree;

pub use usage::{never_used_offenses, used_once_offenses};

pub struct ZigFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<crate::scope_model::Scope>,
}

impl ZigFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> ZigFile<'_> {
    let scopes = scope::collect(tree.root_node(), src);
    ZigFile { src, tree, scopes }
}

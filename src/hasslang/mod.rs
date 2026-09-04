//! Haskell-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust/Zig backends' semantics where a direct
//! analogue exists):
//! - Units: every value-level `function` (has a `match`) and every
//!   top-level / class / instance `bind` with a `match`. Nested
//!   where-bound `function`s are separate units. Local `let`/`where`
//!   binds and lambdas roll into the enclosing unit.
//! - A: local named binds (+1), do/`<-` pattern binders, generator and
//!   pattern-guard binders, case-alternative and lambda pattern
//!   binders, as-patterns. Unit parameters are protocol (not A).
//! - B: applications (`apply`), non-condition `infix` operators,
//!   unary `negation`.
//! - C: `conditional`, each `alternative`, boolean/pattern guards,
//!   multi-way-if matches, comparisons and `&&`/`||`.
//! - UsedOnce: single plain write, pure RHS, straight-line write,
//!   single read after the write. Parameters and pattern binders are
//!   protocol. Root-scope (module) bindings stay unreported
//!   (`include_root_scope: false`) because exports may be consumed
//!   elsewhere.
//! - NeverUsed: written but never read, reported at the first write.
//!   Parameters and pattern binders stay exempt (`exempt_bindings`),
//!   matching the man-page contract.

pub(crate) mod abc;
mod nodes;
mod patterns;
mod scope;
mod usage;

#[cfg(test)]
mod tests;

use tree_sitter::Tree;

pub use usage::{never_used_offenses, used_once_offenses};

pub struct HsFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<crate::scope_model::Scope>,
}

impl HsFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> HsFile<'_> {
    HsFile {
        src,
        scopes: scope::collect(tree.root_node(), src),
        tree,
    }
}

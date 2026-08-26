//! Go-language backend: scope model, AbcSize, used-once/never-used.
//!
//! Metric spec (mirrors the Rust backend's semantics where a direct
//! analogue exists):
//! - Units: every named `function_declaration` and `method_declaration`.
//!   Function literals roll into the enclosing unit (mirrors Ruby blocks /
//!   Rust closures) -- they are NOT unit boundaries here.
//! - A: short var declarations and assignments (+1 per written identifier
//!   target), var specs, inc/dec statements.
//! - B: calls, arithmetic/bitwise/shift binary operators, unary operators
//!   (including `<-` receives and pointer indirections).
//! - C: if / for (all forms), each switch/select/type-switch case,
//!   comparisons and `&&`/`||`.
//! - UsedOnce: single plain write, pure RHS, straight-line write, single
//!   read after the write. Parameters, results, blank `_` and
//!   underscore-free-only rule apply; struct fields (`field_identifier`)
//!   are never variable reads.
//! - NeverUsed: written but never read, reported at the first write;
//!   same exclusions.

mod abc;
mod vars;


use tree_sitter::Tree;

pub use vars::{never_used_offenses, used_once_offenses};

pub struct GoFile<'t> {
    pub src: &'t [u8],
    pub tree: Tree,
    scopes: Vec<vars::Scope>,
}

impl GoFile<'_> {
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

pub fn build(src: &[u8], tree: Tree) -> GoFile<'_> {
    let scopes = vars::collect(tree.root_node(), src);
    GoFile { src, tree, scopes }
}

pub fn analyze(fm: &GoFile, max: f64) -> Vec<crate::abc::AbcOffense> {
    let mut offenses = abc::all_scores(fm);
    offenses.retain(|o| o.score > max);
    offenses
}

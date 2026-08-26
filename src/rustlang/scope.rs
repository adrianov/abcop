//! Scope tree and per-identifier bookkeeping shared by the analyses.

use std::collections::HashMap;

use tree_sitter::{Node, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IntroKind {
    /// `let x = ...`
    Assign,
    /// params, `+=`, pattern bindings — never inline candidates
    Binding,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Write {
    pub(super) byte: usize,
    pub(super) node_id: usize,
    pub(super) plain: bool,
    pub(super) rhs: Option<(usize, usize)>,
}

#[derive(Debug)]
pub(super) struct Entry {
    pub(super) intro_byte: usize,
    pub(super) intro_kind: IntroKind,
    pub(super) writes: Vec<Write>,
    pub(super) reads: Vec<usize>,
    /// reads that occurred inside a `token_tree` (macro input): macros may
    /// give identifiers syntactic roles, so they never justify inlining
    pub(super) macro_reads: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScopeKind {
    Root,
    Function,
    Block,
}

pub(super) struct Scope {
    pub(super) parent: Option<usize>,
    pub(super) kind: ScopeKind,
    pub(super) entries: HashMap<Box<str>, Entry>,
}

pub struct RustFile<'s> {
    pub src: &'s [u8],
    pub tree: Tree,
    pub(super) scopes: Vec<Scope>,
}

impl<'s> RustFile<'s> {
    pub(super) fn text(&self, n: Node<'_>) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

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

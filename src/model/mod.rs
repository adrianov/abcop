//! Single-file semantic model: scope tree, local-variable introductions,
//! writes and reads. Shared by the ABC calculator (safe-nav receiver
//! classification) and the used-once / never-used detectors.
//!
//! Submodules: [`builder`] (the tree-walking model constructor) with its
//! [`writes`] / [`reads`] / [`scopes`] handler groups, [`masgn`] for
//! multiple-assignment targets, and [`lifetime`] for write observation
//! (which values are read before the next sure overwrite).

mod builder;
mod lifetime;
mod reads;
mod scopes;
mod writes;

mod masgn;

use std::collections::HashMap;

use tree_sitter::{Node, Tree};

pub type ScopeId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntroKind {
    /// plain `x = ...`
    Assign,
    /// `x op= ...` / masgn target / block param etc.
    Binding,
}

#[derive(Clone, Copy, Debug)]
pub struct Write {
    pub byte: usize,
    pub node_id: usize,
    pub kind: WriteKind,
    /// RHS expression of a plain assignment as `(node id, end byte)`.
    pub rhs: Option<(usize, usize)>,
    /// False when the write sits under `if`/`rescue`/loop — it may not run.
    pub unconditional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Plain,
    OpAssign,
    Masgn,
    ForVar,
    RescueVar,
}

#[derive(Clone, Copy, Debug)]
pub struct Read {
    pub byte: usize,
    pub under_defined: bool,
}

#[derive(Debug)]
pub struct Entry {
    pub intro_byte: usize,
    pub intro_kind: IntroKind,
    pub writes: Vec<Write>,
    pub reads: Vec<Read>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Root,
    Method,
    ClassLike,
    Block,
}

#[derive(Debug)]
pub struct ScopeData {
    pub parent: Option<ScopeId>,
    pub kind: ScopeKind,
    /// Byte offset at which this scope was entered; used when climbing out of
    /// a block: parent bindings are only shared if introduced before entry.
    pub entered_at: usize,
    pub entries: HashMap<Box<str>, Entry>,
}

pub struct FileModel<'s> {
    pub src: &'s [u8],
    pub tree: Tree,
    pub scopes: Vec<ScopeData>,
    /// safe-navigation sites whose receiver resolved to a local var:
    /// `(receiver start byte, receiver name, owning scope)`
    pub csend_sites: Vec<(usize, Box<str>, ScopeId)>,
    /// bare identifiers that did NOT resolve to locals — zero-arity method
    /// calls (parser-gem `:send` vcalls). Excludes `__FILE__`/`__LINE__`/`__ENCODING__`.
    pub vcall_sites: Vec<usize>,
}

impl<'s> FileModel<'s> {
    /// Text accessor whose lifetime is bound to the model, not the node.
    pub fn text(&self, n: Node<'_>) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    /// Resolve `name` visible at byte `pos` starting from scope `s`.
    #[allow(dead_code)]
    pub fn lookup(&self, s: ScopeId, pos: usize, name: &str) -> Option<ScopeId> {
        lookup_scope(&self.scopes, s, pos, name)
    }

    pub fn line_col(&self, byte: usize) -> (usize, usize) {
        let point = self
            .tree
            .root_node()
            .descendant_for_byte_range(byte, byte)
            .map(|n| n.start_position())
            .unwrap_or_default();
        (point.row + 1, point.column)
    }
}

/// Scope resolution so the in-progress builder can query visibility too.
pub fn lookup_scope(
    scopes: &[ScopeData],
    mut s: ScopeId,
    pos: usize,
    name: &str,
) -> Option<ScopeId> {
    let mut effective_pos = pos;
    loop {
        let scope = &scopes[s];
        if let Some(e) = scope.entries.get(name)
            && e.intro_byte <= effective_pos
        {
            return Some(s);
        }
        let parent = scope.parent?;
        // Climbing out of a nested block: only bindings introduced before the
        // boundary are shared with the outer scope.
        if matches!(scope.kind, ScopeKind::Block) {
            effective_pos = effective_pos.min(scope.entered_at);
        } else {
            // method/class scopes are opaque to locals from above
            return None;
        }
        s = parent;
    }
}

pub fn build<'s>(src: &'s [u8], tree: Tree) -> FileModel<'s> {
    let mut scopes = vec![ScopeData {
        parent: None,
        kind: ScopeKind::Root,
        entered_at: 0,
        entries: HashMap::new(),
    }];
    let mut csend_sites = Vec::new();
    let mut vcall_sites = Vec::new();
    {
        let mut b = builder::Builder {
            src,
            scopes: &mut scopes,
            csend_sites: &mut csend_sites,
            vcall_sites: &mut vcall_sites,
        };
        b.walk(tree.root_node(), 0, false);
    }
    FileModel {
        src,
        tree,
        scopes,
        csend_sites,
        vcall_sites,
    }
}

/// Build a model from in-memory source; test helper shared by backends.
#[cfg(test)]
pub(crate) fn build_from_str(src: &str) -> FileModel<'_> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .expect("ruby grammar");

    build(
        src.as_bytes(),
        parser.parse(src, None).expect("syntax tree"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebind_inside_block_hits_shared_outer_binding() {
        assert_eq!(
            build_from_str("x = 1\n[1].each { x = 2 }\n").scopes[0].entries["x"]
                .writes
                .len(),
            2
        );
    }

    #[test]
    fn vcall_never_counts_as_variable_read() {
        let fm = build_from_str("def m\n  bar\nend\n");
        assert!(fm.scopes.iter().all(|s| s.entries.is_empty()));
    }

    #[test]
    fn local_read_after_introduction_is_tracked() {
        assert_eq!(
            build_from_str("def m\n  x = 1\n  p x\nend\n")
                .scopes
                .into_iter()
                .find(|s| s.kind == ScopeKind::Method)
                .expect("method scope")
                .entries
                .remove("x")
                .map(|e| (e.writes.len(), e.reads.len()))
                .expect("entry"),
            (1, 1)
        );
    }

    #[test]
    fn block_local_var_does_not_leak() {
        let fm = build_from_str("[1].each { y = 2 }\n");
        let blk = fm
            .scopes
            .iter()
            .find(|s| s.kind == ScopeKind::Block)
            .expect("block scope");
        assert!(blk.entries.contains_key("y"));
        assert!(!fm.scopes[0].entries.contains_key("y"));
    }
}

//! Variable write/read model powering UsedOnce/NeverUsed for Python.
//!
//! Scope rules mirror the Rust backend: reads resolve through Block
//! scopes (lambda bodies roll up like Rust closures) but stop at
//! Function/Class boundaries (nested defs are independent scopes). Reads
//! before the binding's introduction position never count -- Python
//! raises UnboundLocalError for exactly that pattern.

mod collector;
mod purity;

use std::collections::HashMap;

use tree_sitter::Node;

use super::PyFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;
use purity::{pure, unconditionally_executed};

pub(super) use collector::collect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// plain assignment / walrus -- inline candidate
    Assign,
    /// augmented assignment or pattern capture -- never a candidate
    Binding,
}

#[derive(Clone, Copy, Debug)]
struct Write {
    byte: usize,
    node_id: usize,
    plain: bool,
    rhs: Option<usize>,
}

struct Entry {
    intro_byte: usize,
    intro_kind: IntroKind,
    writes: Vec<Write>,
    reads: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Root,
    Function,
    Class,
    Block,
}

pub(super) struct Scope {
    parent: Option<usize>,
    kind: ScopeKind,
    entries: HashMap<Box<str>, Entry>,
}

/// Statements whose subtrees carry no variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "parameters",
    "import_statement",
    "import_from_statement",
    "future_import_statement",
    "global_statement",
    "nonlocal_statement",
];

/// Control-flow ancestors that disqualify a write from inlining.
const VETO_KINDS: &[&str] = &[
    "if_statement",
    "elif_clause",
    "else_clause",
    "for_statement",
    "while_statement",
    "try_statement",
    "except_clause",
    "finally_clause",
    "match_statement",
    "case_clause",
    "with_statement",
    "conditional_expression",
];

pub(crate) fn used_once_offenses(fm: &PyFile) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if let Some(o) = single_use(fm, &nodes, name, e) {
                out.push(o);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

pub(crate) fn never_used_offenses(fm: &PyFile) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if !e.reads.is_empty() || e.writes.is_empty() {
                continue;
            }
            let (line, column) = fm.line_col(e.writes[0].byte);
            out.push(NeverUsedOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

/// An entry is an inline candidate when it holds one plain
/// non-augmented write carrying an RHS, followed by exactly one later
/// read.
fn inline_candidate(e: &Entry) -> bool {
    e.intro_kind == IntroKind::Assign
        && e.writes.len() == 1
        && e.reads.len() == 1
        && e.writes[0].plain
        && e.writes[0].rhs.is_some()
        && e.reads[0] > e.writes[0].byte
}

fn single_use<'t>(
    fm: &'t PyFile<'t>,
    nodes: &HashMap<usize, Node<'t>>,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    if !inline_candidate(e) {
        return None;
    }
    let (rhs, write_node) = inline_nodes(nodes, e)?;
    if !pure(rhs) || !unconditionally_executed(write_node) {
        return None;
    }
    let (line, column) = fm.line_col(e.writes[0].byte);
    Some(UsedOnceOffense {
        line,
        column,
        name: name.to_string(),
    })
}

/// RHS and written-identifier nodes of an entry's first write.
fn inline_nodes<'t>(nodes: &HashMap<usize, Node<'t>>, e: &Entry) -> Option<(Node<'t>, Node<'t>)> {
    let w = &e.writes[0];
    let rhs = *nodes.get(&w.rhs?)?;
    let write_node = *nodes.get(&w.node_id)?;
    Some((rhs, write_node))
}

fn index_nodes<'t>(root: Node<'t>) -> HashMap<usize, Node<'t>> {
    let mut map = HashMap::new();
    rec(root, &mut map);
    map
}

fn rec<'t>(n: Node<'t>, map: &mut HashMap<usize, Node<'t>>) {
    map.insert(n.id(), n);
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        rec(child, map);
    }
}

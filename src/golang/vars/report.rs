//! Diagnostics derived from the scope model: UsedOnce inline candidates
//! and NeverUsed writes.

use std::collections::HashMap;

use tree_sitter::Node;

use super::purity::{index_nodes, pure, unconditionally_executed};
use super::{Entry, IntroKind, Write};
use crate::golang::GoFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

pub fn used_once_offenses(fm: &GoFile) -> Vec<UsedOnceOffense> {
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

pub fn never_used_offenses(fm: &GoFile) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if let Some(o) = never_used(fm, name, e) {
                out.push(o);
            }
        }
    }
    finalize(out)
}

/// Sort by position and drop duplicates reported across overlapping scopes.
fn finalize(mut out: Vec<NeverUsedOffense>) -> Vec<NeverUsedOffense> {
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

fn never_used(fm: &GoFile, name: &str, e: &Entry) -> Option<NeverUsedOffense> {
    if !e.reads.is_empty() || e.writes.is_empty() {
        return None;
    }
    let first = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
    let (line, column) = fm.line_col(first);
    Some(NeverUsedOffense {
        line,
        column,
        name: name.to_string(),
    })
}

/// Structurally eligible single write/read pair: one plain write with a
/// RHS link and one later read.
fn candidate(e: &Entry) -> Option<&Write> {
    if e.intro_kind != IntroKind::Assign || e.writes.len() != 1 || e.reads.len() != 1 {
        return None;
    }
    let w = &e.writes[0];
    if !w.plain || w.rhs.is_none() || e.reads[0] <= w.byte {
        return None;
    }
    Some(w)
}

fn single_use<'t>(
    fm: &'t GoFile<'t>,
    nodes: &HashMap<usize, Node<'t>>,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    let w = candidate(e)?;
    let rhs = *nodes.get(&w.rhs?)?;
    let write_node = *nodes.get(&w.node_id)?;
    if !pure(rhs) || !unconditionally_executed(write_node) {
        return None;
    }
    let (line, column) = fm.line_col(w.byte);
    Some(UsedOnceOffense {
        line,
        column,
        name: name.to_string(),
    })
}

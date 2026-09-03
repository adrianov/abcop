//! Diagnostics derived from the scope model: UsedOnce inline candidates
//! and NeverUsed writes.

use std::collections::HashMap;

use tree_sitter::Node;

use super::purity::{index_nodes, inlinable_rhs, keep_init, unconditionally_executed};
use super::{Entry, IntroKind, Write};
use crate::golang::GoFile;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

pub fn used_once_offenses(fm: &GoFile) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();
    for (scope, scope_data) in fm.scopes.iter().enumerate() {
        for (name, e) in &scope_data.entries {
            if let Some(o) = single_use(fm, &nodes, scope, name, e) {
                out.push(o);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

pub fn never_used_offenses(fm: &GoFile) -> Vec<NeverUsedOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();
    for (scope, scope_data) in fm.scopes.iter().enumerate() {
        for (name, e) in &scope_data.entries {
            if let Some(o) = never_used(fm, &nodes, scope, name, e) {
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

fn never_used(
    fm: &GoFile,
    nodes: &HashMap<usize, Node>,
    scope: usize,
    name: &str,
    e: &Entry,
) -> Option<NeverUsedOffense> {
    if !e.reads.is_empty() || e.writes.is_empty() {
        return None;
    }

    let byte = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
    Some(NeverUsedOffense {
        line: fm.line_col(byte).0,
        column: fm.line_col(byte).1,
        name: name.to_string(),
        keep_init: keep_init_for_dead(fm, nodes, scope, e),
    })
}

fn keep_init_for_dead(fm: &GoFile, nodes: &HashMap<usize, Node>, scope: usize, e: &Entry) -> bool {
    let w = match plain_write(e) {
        Some(w) => w,
        None => return false,
    };
    let (rhs, write_node) = match write_rhs_nodes(w, nodes) {
        Some(nodes) => nodes,
        None => return false,
    };
    inlinable_at_write(fm, w, rhs, write_node, scope, None).is_some() && keep_init(rhs)
}

fn write_rhs_nodes<'t>(
    w: &Write,
    nodes: &HashMap<usize, Node<'t>>,
) -> Option<(Node<'t>, Node<'t>)> {
    Some((*nodes.get(&w.rhs?)?, *nodes.get(&w.node_id)?))
}

fn inlinable_at_write(
    fm: &GoFile,
    w: &Write,
    rhs: Node,
    write_node: Node,
    scope: usize,
    read_byte: Option<usize>,
) -> Option<()> {
    if inlinable_rhs(
        fm.src,
        &fm.scopes,
        rhs,
        scope,
        w.byte,
        read_byte,
        Some(write_node),
    ) && unconditionally_executed(write_node)
    {
        Some(())
    } else {
        None
    }
}

fn plain_write(e: &Entry) -> Option<&Write> {
    e.writes.iter().find(|w| w.plain && w.rhs.is_some())
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
    scope: usize,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    let w = candidate(e)?;
    let (rhs, write_node) = write_rhs_nodes(w, nodes)?;
    inlinable_at_write(fm, w, rhs, write_node, scope, Some(e.reads[0]))?;
    Some(UsedOnceOffense {
        line: fm.line_col(w.byte).0,
        column: fm.line_col(w.byte).1,
        name: name.to_string(),
    })
}

//! UsedOnce analysis: single-use plain `let` bindings that can be inlined.

use std::collections::HashMap;

use tree_sitter::Node;

use super::pure::{pure, unconditionally_executed};
use super::scope::{Entry, IntroKind, RustFile, Write};


use crate::used_once::UsedOnceOffense;
pub fn used_once_offenses(fm: &RustFile) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();

    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if let Some(offense) = single_use(fm, &nodes, name, e) {
                out.push(offense);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
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

/// One plain `let`, one later read, no macro reads, pure RHS and
/// straight-line execution: the read can be inlined into the write.
fn single_use<'t>(
    fm: &RustFile,
    nodes: &HashMap<usize, Node<'t>>,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    if !single_plain_use(e) {
        return None;
    }
    let w = &e.writes[0];
    let (rhs_node, write_node) = resolved_rhs(nodes, w)?;
    if !inlinable_write(fm, rhs_node, write_node) {
        return None;
    }
    Some(offense_at_write(fm, name, w.byte))
}

/// Pure RHS executed straight-line: inlining preserves behaviour.
fn inlinable_write(fm: &RustFile, rhs_node: Node, write_node: Node) -> bool {
    pure(fm, rhs_node) && unconditionally_executed(write_node)
}

fn offense_at_write(fm: &RustFile, name: &str, byte: usize) -> UsedOnceOffense {
    let (line, column) = fm.line_col(byte);
    UsedOnceOffense {
        line,
        column,
        name: name.to_string(),
    }
}

/// Assign-introduced entry with exactly one plain write, exactly one read
/// after it, and no macro-interpolated reads.
fn single_plain_use(e: &Entry) -> bool {
    if e.intro_kind != IntroKind::Assign || e.writes.len() != 1 || e.reads.len() != 1 {
        return false;
    }
    let w = e.writes[0];
    w.plain && e.reads[0] > w.byte && e.macro_reads == 0
}

/// Tree nodes for a write's RHS expression and the write itself.
fn resolved_rhs<'t>(nodes: &HashMap<usize, Node<'t>>, w: &Write) -> Option<(Node<'t>, Node<'t>)> {
    let (rhs_id, _) = w.rhs?;
    Some((*nodes.get(&rhs_id)?, *nodes.get(&w.node_id)?))
}

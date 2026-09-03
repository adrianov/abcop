//! UsedOnce analysis: single-use plain `let` bindings that can be inlined.

use std::collections::HashMap;

use tree_sitter::Node;

use super::pure::{inlinable_rhs, unconditionally_executed};
use super::scope::{Entry, IntroKind, RustFile, Write};

use crate::used_once::UsedOnceOffense;
pub fn used_once_offenses(fm: &RustFile) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();

    for (scope, scope_data) in fm.scopes.iter().enumerate() {
        for (name, e) in &scope_data.entries {
            if let Some(offense) = single_use(fm, &nodes, scope, name, e) {
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
    scope: usize,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    if !single_plain_use(e) {
        return None;
    }
    let w = &e.writes[0];
    let (rhs_node, write_node) = resolved_rhs(nodes, w)?;
    if !inlinable_rhs(fm, rhs_node, scope, w.byte, Some(e.reads[0]), Some(write_node))
        || !unconditionally_executed(write_node)
        || reborrow_blocks(fm, rhs_node, e.reads[0])
    {
        return None;
    }
    Some(offense_at_write(fm, name, w.byte))
}

/// `let s = b.foo(); use(b, s)` / `let s = self.foo(); self.bar(s)` cannot be
/// inlined: the call that produced `s` and the call that consumes it both
/// need the same receiver.
fn reborrow_blocks(fm: &RustFile, rhs: Node, read_byte: usize) -> bool {
    let Some(root) = call_root_name(fm, rhs) else {
        return false;
    };
    fm.tree
        .root_node()
        .descendant_for_byte_range(read_byte, read_byte)
        .and_then(enclosing_call)
        .is_some_and(|call| call_mentions_name(fm, call, &root))
}

fn call_root_name(fm: &RustFile, n: Node) -> Option<String> {
    match n.kind() {
        "call_expression" => call_root_through(fm, n, "function"),
        "method_call_expression" => call_root_through(fm, n, "receiver"),
        "field_expression" => call_root_through(fm, n, "value"),
        "await_expression" => call_root_name(fm, n.named_child(0)?),
        "self" | "identifier" => Some(fm.text(n).to_string()),
        _ => None,
    }
}

fn call_root_through(fm: &RustFile, n: Node, field: &str) -> Option<String> {
    call_root_name(fm, n.child_by_field_name(field)?)
}

fn enclosing_call(read: Node) -> Option<Node> {
    let mut cur = read;
    while let Some(parent) = cur.parent() {
        if parent.kind() == "arguments" {
            return parent.parent();
        }
        if parent.kind() == "let_declaration" || parent.kind() == "function_item" {
            return None;
        }
        cur = parent;
    }
    None
}

fn call_mentions_name(fm: &RustFile, call: Node, name: &str) -> bool {
    if call_root_name(fm, call).as_deref() == Some(name) {
        return true;
    }
    let Some(args) = call.child_by_field_name("arguments") else {
        return false;
    };
    args.children(&mut args.walk()).any(|ch| {
        ch.is_named() && ((ch.kind() == "identifier" && fm.text(ch) == name) || call_root_name(fm, ch).as_deref() == Some(name))
    })
}

fn offense_at_write(fm: &RustFile, name: &str, byte: usize) -> UsedOnceOffense {
    UsedOnceOffense {
        line: fm.line_col(byte).0,
        column: fm.line_col(byte).1,
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

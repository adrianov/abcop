//! RHS inlinability predicates and NeverUsed analysis.

use tree_sitter::Node;

use crate::inlinable::{immediate_substitutable, keep_init_kind, RUST_IDENT, RUST_UNITS};

use super::scope::{Entry, RustFile, Scope, ScopeKind, Write};

/// Conservative RHS purity: literals, constant paths, and compositions of
/// comparisons/logical/arithmetic over those. Calls, macros, `?`, field reads
/// through non-const bases, and local-variable references are all rejected.
pub(super) fn pure(fm: &RustFile, n: Node) -> bool {
    match n.kind() {
        "integer_literal" | "float_literal" | "char_literal" | "string_literal"
        | "raw_string_literal" | "true" | "false" | "unit_type" => true,
        "scoped_identifier" => true, // constants; enforced immutable by rustc
        "reference_expression" | "unary_expression" => children_pure(fm, n),
        "binary_expression" | "tuple_expression" | "array_expression" | "range_expression" => {
            children_pure(fm, n)
        }
        "type_cast_expression" => child_pure(fm, n),
        _ => false,
    }
}

pub(super) fn inlinable_rhs(
    fm: &RustFile,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
    write_site: Option<Node>,
) -> bool {
    if RUST_UNITS.contains(&n.kind()) {
        return match read_byte {
            None => true,
            Some(rb) => write_site.is_some_and(|site| immediate_substitutable(site, rb)),
        };
    }
    if n.kind() == RUST_IDENT {
        let name = fm.text(n);
        return match read_byte {
            Some(end) => alias_stable(fm, scope, write_byte, name, write_byte, end),
            None => true,
        };
    }
    pure(fm, n)
}

pub(super) fn keep_init(n: Node) -> bool {
    keep_init_kind(n, RUST_UNITS)
}

fn alias_stable(
    fm: &RustFile,
    scope: usize,
    pos: usize,
    name: &str,
    write_byte: usize,
    read_byte: usize,
) -> bool {
    let Some(bind_scope) = lookup(&fm.scopes, scope, pos, name) else {
        return true;
    };
    let Some(entry) = fm.scopes[bind_scope].entries.get(name) else {
        return true;
    };
    !entry
        .writes
        .iter()
        .any(|w| w.byte > write_byte && w.byte < read_byte)
}

fn lookup(scopes: &[Scope], scope: usize, pos: usize, name: &str) -> Option<usize> {
    let data = &scopes[scope];
    if let Some(e) = data.entries.get(name) {
        return if e.intro_byte <= pos {
            Some(scope)
        } else {
            None
        };
    }
    match data.kind {
        ScopeKind::Block => lookup(scopes, data.parent?, pos, name),
        _ => None,
    }
}

fn children_pure<'t>(fm: &RustFile, n: Node<'t>) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(fm, ch))
}

/// Purity of the node's `value` field (casts and field reads).
fn child_pure<'t>(fm: &RustFile, n: Node<'t>) -> bool {
    n.child_by_field_name("value")
        .map(|v| pure(fm, v))
        .unwrap_or(false)
}

/// Straight-line execution check up to the nearest function/closure/block
/// boundary (the binding's scope).
pub(super) fn unconditionally_executed(write_node: Node) -> bool {
    const VETO: [&str; 7] = [
        "if_expression",
        "if_let_expression",
        "while_expression",
        "while_let_expression",
        "for_expression",
        "match_arm",
        "match_expression",
    ];
    const OWNERS: [&str; 3] = ["function_item", "closure_expression", "block"];
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if VETO.contains(&n.kind()) {
            return false;
        }
        if OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

/// NeverUsed for Rust sources: bindings with writes but zero reads whose
/// initializer carries no macro invocation.
pub fn never_used_offenses(fm: &RustFile) -> Vec<crate::never_used::NeverUsedOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();
    for (scope, scope_data) in fm.scopes.iter().enumerate() {
        for (name, e) in &scope_data.entries {
            if let Some(offense) = dead_offense(fm, &nodes, scope, name, e) {
                out.push(offense);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

fn dead_offense(
    fm: &RustFile,
    nodes: &std::collections::HashMap<usize, Node>,
    scope: usize,
    name: &str,
    e: &Entry,
) -> Option<crate::never_used::NeverUsedOffense> {
    if !never_used_entry(fm, e) {
        return None;
    }
    let first = e.writes.iter().map(|w| w.byte).min()?;
    Some(offense_at_write(
        fm,
        name,
        first,
        keep_init_for_dead(fm, nodes, scope, e),
    ))
}

fn keep_init_for_dead(
    fm: &RustFile,
    nodes: &std::collections::HashMap<usize, Node>,
    scope: usize,
    e: &Entry,
) -> bool {
    let w = match plain_write(e) {
        Some(w) => w,
        None => return false,
    };
    if !macro_free_rhs(fm, w) {
        return false;
    }
    let (rhs, write_node) = match write_rhs_nodes(w, nodes) {
        Some(nodes) => nodes,
        None => return false,
    };
    inlinable_rhs(fm, rhs, scope, w.byte, None, None) && unconditionally_executed(write_node)
        && keep_init(rhs)
}

fn write_rhs_nodes<'t>(
    w: &Write,
    nodes: &std::collections::HashMap<usize, Node<'t>>,
) -> Option<(Node<'t>, Node<'t>)> {
    let (rhs_id, _) = w.rhs?;
    Some((*nodes.get(&rhs_id)?, *nodes.get(&w.node_id)?))
}

/// Zero reads overall and no macro-interpolated initializer anywhere.
fn never_used_entry(fm: &RustFile, e: &Entry) -> bool {
    e.reads.is_empty() && !e.writes.is_empty() && e.writes.iter().all(|w| macro_free_rhs(fm, w))
}

fn plain_write(e: &Entry) -> Option<&Write> {
    e.writes.iter().find(|w| w.plain && w.rhs.is_some())
}

fn index_nodes<'t>(root: Node<'t>) -> std::collections::HashMap<usize, Node<'t>> {
    let mut map = std::collections::HashMap::new();
    rec(root, &mut map);
    map
}

fn rec<'t>(n: Node<'t>, map: &mut std::collections::HashMap<usize, Node<'t>>) {
    map.insert(n.id(), n);
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        rec(child, map);
    }
}

/// A write qualifies when its RHS subtree contains no macro invocation.
fn macro_free_rhs(fm: &RustFile, w: &Write) -> bool {
    let Some((_, byte)) = w.rhs else {
        return true;
    };
    fm.tree
        .root_node()
        .descendant_for_byte_range(byte, byte)
        .map(|node| !contains_macro(node))
        .unwrap_or(true)
}

fn offense_at_write(
    fm: &RustFile,
    name: &str,
    byte: usize,
    keep_init: bool,
) -> crate::never_used::NeverUsedOffense {
    let (line, column) = fm.line_col(byte);
    crate::never_used::NeverUsedOffense {
        line,
        column,
        name: name.to_string(),
        keep_init,
    }
}

fn contains_macro(n: Node) -> bool {
    let mut cursor = n.walk();
    n.children(&mut cursor)
        .any(|c| c.kind() == "macro_invocation" || contains_macro(c))
}

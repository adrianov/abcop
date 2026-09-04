//! Candidate evaluation over a collected scope tree: which bindings are
//! inlineable (UsedOnce) and which are dead writes (NeverUsed), driven
//! by a per-language [`Semantics`].

use std::collections::HashMap;

use tree_sitter::Node;

use super::{Entry, IntroKind, Scope, Write};
use crate::inlinable::{keep_init_rhs, rhs_inlinable};
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

/// The parts of candidate evaluation that differ per language.
pub struct Semantics {
    /// Literals and operator compositions over them.
    pub pure: fn(Node) -> bool,
    /// Expression kinds that move as one unit (calls, member/index chains).
    pub unit_kinds: &'static [&'static str],
    /// AST kind for a bare local read on the RHS.
    pub ident_kind: &'static str,
    /// Ancestors that mark a write as conditional (kills inlining).
    pub veto: &'static [&'static str],
    /// Ancestors that end the straight-line check (unit boundaries).
    pub owners: &'static [&'static str],
    /// Analyze bindings living directly in the Root scope? Module-level
    /// constants may be consumed by other files, which single-file
    /// analysis cannot see -- backends for such languages set this to
    /// `false` to keep the rules free of cross-file false positives.
    pub include_root_scope: bool,
    /// When true, [`IntroKind::Binding`] entries (parameters, pattern
    /// heads, catch/payload binders) are omitted from NeverUsed — the
    /// man-page exemption. Languages that still flag unread parameters
    /// leave this false.
    pub exempt_bindings: bool,
}

/// Inline candidates: one plain introduction, one later resolved read,
/// inlinable RHS, written on a straight-line path.
pub fn used_once_offenses(
    root: Node,
    src: &[u8],
    line_col: &dyn Fn(usize) -> (usize, usize),
    scopes: &[Scope],
    sem: &Semantics,
) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(root);
    let mut out = Vec::new();
    for (scope, scope_data) in scopes.iter().enumerate() {
        if !sem.include_root_scope && scope_data.kind == super::ScopeKind::Root {
            continue;
        }
        for (name, e) in &scope_data.entries {
            if let Some(offense) =
                candidate_offense(name, e, &nodes, src, scopes, scope, sem, line_col)
            {
                out.push(offense);
            }
        }
    }
    finish(out)
}

fn candidate_offense(
    name: &str,
    e: &super::Entry,
    nodes: &HashMap<usize, Node>,
    src: &[u8],
    scopes: &[Scope],
    scope: usize,
    sem: &Semantics,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> Option<UsedOnceOffense> {
    let w = candidate(e)?;
    let (rhs, write_node) = write_rhs_nodes(w, nodes)?;
    inlinable_write(src, w, rhs, write_node, scopes, scope, sem, Some(e.reads[0]))?;
    let (line, column) = line_col(w.byte);
    Some(UsedOnceOffense {
        line,
        column,
        name: name.to_string(),
    })
}

fn write_rhs_nodes<'t>(
    w: &Write,
    nodes: &HashMap<usize, Node<'t>>,
) -> Option<(Node<'t>, Node<'t>)> {
    let rhs_id = w.rhs?;
    Some((*nodes.get(&rhs_id)?, *nodes.get(&w.node_id)?))
}

fn inlinable_write(
    src: &[u8],
    w: &Write,
    rhs: Node,
    write_node: Node,
    scopes: &[Scope],
    scope: usize,
    sem: &Semantics,
    read_byte: Option<usize>,
) -> Option<()> {
    if rhs_inlinable(
        src,
        rhs,
        sem,
        scopes,
        scope,
        w.byte,
        read_byte,
        Some(write_node),
    ) && straight_line(write_node, sem)
    {
        Some(())
    } else {
        None
    }
}

fn candidate(e: &Entry) -> Option<&Write> {
    if e.intro_kind != IntroKind::Assign || e.writes.len() != 1 || e.reads.len() != 1 {
        return None;
    }
    let w = &e.writes[0];
    (w.plain && e.reads[0] > w.byte).then_some(w)
}

fn straight_line(write_node: Node, sem: &Semantics) -> bool {
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if sem.veto.contains(&n.kind()) {
            return false;
        }
        if sem.owners.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

/// Dead writes: bindings with at least one write and no resolved read,
/// reported once at the first write. [`NeverUsedOffense::keep_init`] marks
/// call-chain initializers that can stand alone as statements.
pub fn never_used_offenses(
    root: Node,
    src: &[u8],
    line_col: &dyn Fn(usize) -> (usize, usize),
    scopes: &[Scope],
    sem: &Semantics,
) -> Vec<NeverUsedOffense> {
    let nodes = index_nodes(root);
    let mut out = Vec::new();
    for (scope, scope_data) in scopes.iter().enumerate() {
        if !sem.include_root_scope && scope_data.kind == super::ScopeKind::Root {
            continue;
        }
        for (name, e) in &scope_data.entries {
            if let Some(offense) =
                dead_offense(name, e, &nodes, src, scopes, scope, sem, line_col)
            {
                out.push(offense);
            }
        }
    }
    finish(out)
}

fn dead_offense(
    name: &str,
    e: &Entry,
    nodes: &HashMap<usize, Node>,
    src: &[u8],
    scopes: &[Scope],
    scope: usize,
    sem: &Semantics,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> Option<NeverUsedOffense> {
    if sem.exempt_bindings && e.intro_kind == IntroKind::Binding {
        return None;
    }
    if !e.reads.is_empty() || e.writes.is_empty() {
        return None;
    }
    let first = e.writes.iter().map(|w| w.byte).min()?;
    let (line, column) = line_col(first);
    Some(NeverUsedOffense {
        line,
        column,
        name: name.to_string(),
        keep_init: keep_init_for_dead(e, nodes, src, scopes, scope, sem),
    })
}

fn keep_init_for_dead(
    e: &Entry,
    nodes: &HashMap<usize, Node>,
    src: &[u8],
    scopes: &[Scope],
    scope: usize,
    sem: &Semantics,
) -> bool {
    let w = match plain_write(e) {
        Some(w) => w,
        None => return false,
    };
    let (rhs, write_node) = match write_rhs_nodes(w, nodes) {
        Some(nodes) => nodes,
        None => return false,
    };
    inlinable_write(src, w, rhs, write_node, scopes, scope, sem, None).is_some()
        && keep_init_rhs(rhs, sem)
}

fn plain_write(e: &Entry) -> Option<&Write> {
    e.writes.iter().find(|w| w.plain && w.rhs.is_some())
}

fn finish<T>(mut out: Vec<T>) -> Vec<T>
where
    T: HasPos + PartialEq,
{
    out.sort_by_key(|o| (o.line(), o.column()));
    out.dedup_by(|a, b| a.line() == b.line() && a.column() == b.column() && a.name() == b.name());
    out
}

trait HasPos {
    fn line(&self) -> usize;
    fn column(&self) -> usize;
    fn name(&self) -> &str;
}

macro_rules! impl_has_pos {
    ($t:ty) => {
        impl HasPos for $t {
            fn line(&self) -> usize {
                self.line
            }
            fn column(&self) -> usize {
                self.column
            }
            fn name(&self) -> &str {
                &self.name
            }
        }
    };
}

impl_has_pos!(UsedOnceOffense);
impl_has_pos!(NeverUsedOffense);

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

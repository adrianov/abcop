//! Candidate evaluation over a collected scope tree: which bindings are
//! inlineable (UsedOnce) and which are dead writes (NeverUsed), driven
//! by a per-language [`Semantics`].

use std::collections::HashMap;

use tree_sitter::Node;

use super::{Entry, IntroKind, Scope, Write};
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

/// The parts of candidate evaluation that differ per language.
pub struct Semantics {
    /// Conservative RHS purity: may this expression be inlined without
    /// changing behavior?
    pub pure: fn(Node) -> bool,
    /// Ancestors that mark a write as conditional (kills inlining).
    pub veto: &'static [&'static str],
    /// Ancestors that end the straight-line check (unit boundaries).
    pub owners: &'static [&'static str],
    /// Analyze bindings living directly in the Root scope? Module-level
    /// constants may be consumed by other files, which single-file
    /// analysis cannot see -- backends for such languages set this to
    /// `false` to keep the rules free of cross-file false positives.
    pub include_root_scope: bool,
}

/// Inline candidates: one plain introduction, one later resolved read,
/// pure RHS, written on a straight-line path.
pub fn used_once_offenses(
    root: Node,
    line_col: &dyn Fn(usize) -> (usize, usize),
    scopes: &[Scope],
    sem: &Semantics,
) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(root);
    let mut out = Vec::new();
    for scope in scopes {
        if !sem.include_root_scope && scope.kind == super::ScopeKind::Root {
            continue;
        }
        for (name, e) in &scope.entries {
            let Some(w) = candidate(e) else {
                continue;
            };
            let Some(rhs_id) = w.rhs else { continue };
            let (Some(rhs), Some(write_node)) = (nodes.get(&rhs_id), nodes.get(&w.node_id))
            else {
                continue;
            };
            if !(sem.pure)(*rhs) || !straight_line(*write_node, sem) {
                continue;
            }
            let (line, column) = line_col(w.byte);
            out.push(UsedOnceOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    finish(out)
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
/// reported once at the first write.
pub fn never_used_offenses(
    line_col: &dyn Fn(usize) -> (usize, usize),
    scopes: &[Scope],
    sem: &Semantics,
) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in scopes {
        if !sem.include_root_scope && scope.kind == super::ScopeKind::Root {
            continue;
        }
        for (name, e) in &scope.entries {
            if !e.reads.is_empty() || e.writes.is_empty() {
                continue;
            }
            let first = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
            let (line, column) = line_col(first);
            out.push(NeverUsedOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    finish(out)
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

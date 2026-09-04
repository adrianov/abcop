//! NeverUsed rule: local variables that are assigned but never read.
//!
//! Complements UsedOnce (which requires exactly one read): here a write's
//! value is never observed. Classic case: no reads at all (once per binding
//! at the first write). Also: a write overwritten before any read, including
//! a trailing unused reassignment after earlier live uses.
//! [`NeverUsedOffense::keep_init`] marks call-chain initializers that can
//! stand alone as statements (classic never-used only).

use std::collections::HashMap;

use tree_sitter::Node;

use crate::inlinable::{RUBY_UNITS, keep_init_kind, ruby_inlinable_rhs};
use crate::model::{Entry, FileModel, Write, WriteKind};

#[derive(PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct NeverUsedOffense {
    pub line: usize,
    pub column: usize,
    pub name: String,
    #[serde(default)]
    pub keep_init: bool,
}

pub fn analyze(fm: &FileModel) -> Vec<NeverUsedOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();
    for (scope, scope_data) in fm.scopes.iter().enumerate() {
        for (name, e) in &scope_data.entries {
            out.extend(dead_bindings(fm, &nodes, scope, name, e));
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

fn dead_bindings(
    fm: &FileModel,
    nodes: &HashMap<usize, Node>,
    scope: usize,
    name: &str,
    e: &Entry,
) -> Vec<NeverUsedOffense> {
    if e.writes.is_empty() {
        return Vec::new();
    }
    if e.reads.is_empty() {
        return vec![offense(
            fm,
            name,
            e.writes.iter().map(|w| w.byte).min().unwrap_or(0),
            keep_init_for_dead(fm, nodes, scope, e),
        )];
    }
    // `defined?(x)` counts as a read for classic never-used, but does not
    // observe a write's value — only value-reads open per-write checking.
    if !e.has_value_read() {
        return Vec::new();
    }
    e.unread_writes()
        .map(|w| offense(fm, name, w.byte, false))
        .collect()
}

fn offense(fm: &FileModel, name: &str, byte: usize, keep_init: bool) -> NeverUsedOffense {
    NeverUsedOffense {
        line: fm.line_col(byte).0,
        column: fm.line_col(byte).1,
        name: name.to_string(),
        keep_init,
    }
}

fn keep_init_for_dead(
    fm: &FileModel,
    nodes: &HashMap<usize, Node>,
    scope: usize,
    e: &Entry,
) -> bool {
    let w = match plain_write(e) {
        Some(w) => w,
        None => return false,
    };
    let (rhs_id, _) = match w.rhs {
        Some(rhs) => rhs,
        None => return false,
    };
    let rhs = match nodes.get(&rhs_id) {
        Some(rhs) => *rhs,
        None => return false,
    };
    let write_node = match nodes.get(&w.node_id) {
        Some(node) => *node,
        None => return false,
    };
    ruby_inlinable_rhs(fm, rhs, scope, w.byte, None, Some(write_node))
        && w.unconditional
        && keep_init_kind(rhs, RUBY_UNITS)
}

fn plain_write(e: &Entry) -> Option<&Write> {
    e.writes
        .iter()
        .find(|w| w.kind == WriteKind::Plain && w.rhs.is_some())
}

fn index_nodes<'t>(root: Node<'t>) -> HashMap<usize, Node<'t>> {
    let mut map = HashMap::new();
    fn rec<'t>(n: Node<'t>, map: &mut HashMap<usize, Node<'t>>) {
        map.insert(n.id(), n);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            rec(child, map);
        }
    }
    rec(root, &mut map);
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::build_from_str;

    fn flags(src: &str) -> Vec<NeverUsedOffense> {
        analyze(&build_from_str(src))
    }

    #[test]
    fn shorthand_hash_arg_counts_as_read() {
        let f = flags("def k\n  user = compute\n  g(user:)\nend\n");
        assert!(f.is_empty(), "shorthand key is a read: {f:?}");
    }

    #[test]
    fn dead_call_chain_keeps_initializer() {
        let f = flags("def k\n  gone = compute()\n  p :done\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "gone");
        assert!(f[0].keep_init);
    }

    #[test]
    fn dead_local_is_flagged_at_write_line() {
        let f = flags("def k\n  gone = compute\n  p :done\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "gone");
        assert_eq!(f[0].line, 2);
        assert!(!f[0].keep_init);
    }

    #[test]
    fn multiple_writes_without_reads_reported_once() {
        let f = flags("def k\n  x = 1\n  x += 2\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 2);
    }

    #[test]
    fn read_variable_not_flagged() {
        let f = flags("def k\n  x = 1\n  p x\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn overwrite_before_read_is_flagged() {
        let f = flags(
            "def k\n  x = create(:a)\n  x = create(:b)\n  use(x)\nend\n",
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].name, "x");
        assert_eq!(f[0].line, 2);
        assert!(!f[0].keep_init);
    }

    #[test]
    fn trailing_unused_reassignment_is_flagged() {
        let f = flags("def k\n  x = 1\n  p x\n  x = 2\nend\n");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].line, 4);
        assert_eq!(f[0].name, "x");
    }

    #[test]
    fn reassignment_that_reads_old_value_is_not_dead() {
        let f = flags("def k\n  x = 1\n  x = x + 1\n  p x\nend\n");
        assert!(f.is_empty(), "RHS read observes prior write: {f:?}");
    }

    #[test]
    fn conditional_overwrite_keeps_prior_write_live() {
        let f = flags(
            "def k(c)\n  x = create(:a)\n  x = create(:b) if c\n  use(x)\nend\n",
        );
        assert!(f.is_empty(), "prior write may be read when condition is false: {f:?}");
    }

    #[test]
    fn underscore_names_exempt() {
        let f = flags("def k\n  _tmp = 1\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn unused_rescue_variable_flagged() {
        let f = flags("def k\n  risky\nrescue => e\n  :handled\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "e");
    }

    #[test]
    fn parameters_never_flagged() {
        let f = flags("def k(unused_arg)\n  :body\nend\n");
        assert!(f.is_empty());
    }
}

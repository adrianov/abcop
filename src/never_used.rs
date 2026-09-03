//! NeverUsed rule: local variables that are assigned but never read.
//!
//! Complements UsedOnce (which requires exactly one read): here the read
//! count is zero. Reported once per binding at the first write.
//! [`NeverUsedOffense::keep_init`] marks call-chain initializers that can
//! stand alone as statements.

use std::collections::HashMap;

use tree_sitter::Node;

use crate::inlinable::{RUBY_IDENT, RUBY_UNITS, keep_init_kind, ruby_alias_stable};
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
            if let Some(offense) = dead_binding(fm, &nodes, scope, name, e) {
                out.push(offense);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

fn dead_binding(
    fm: &FileModel,
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
    inlinable_rhs(fm, rhs, scope, w.byte, None)
        && unconditionally_executed(write_node)
        && keep_init_kind(rhs, RUBY_UNITS)
}

fn plain_write(e: &Entry) -> Option<&Write> {
    e.writes
        .iter()
        .find(|w| w.kind == WriteKind::Plain && w.rhs.is_some())
}

fn inlinable_rhs(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
) -> bool {
    if RUBY_UNITS.contains(&n.kind()) {
        return true;
    }
    if n.kind() == RUBY_IDENT {
        return match read_byte {
            Some(end) => ruby_alias_stable(fm, scope, fm.text(n), write_byte, end),
            None => true,
        };
    }
    pure(fm, n)
}

fn pure(fm: &FileModel, n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "true" | "false" | "nil" | "simple_symbol" | "symbol" | "self"
        | "constant" => true,
        "string" => string_without_interpolation(n),
        "array" | "range" | "binary" => all_named_pure(fm, n),
        "hash" => hash_pure(fm, n),
        "unary" => unary_pure(fm, n),
        "parenthesized_statements" => paren_pure(fm, n),
        _ => false,
    }
}

fn named_children<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    n.children(&mut n.walk()).filter(|c| c.is_named()).collect()
}

fn all_named_pure(fm: &FileModel, n: Node) -> bool {
    named_children(n).into_iter().all(|c| pure(fm, c))
}

fn string_without_interpolation(n: Node) -> bool {
    !n.children(&mut n.walk())
        .any(|c| c.kind() == "interpolation")
        && n.child_by_field_name("interpolation").is_none()
}

fn hash_pure(fm: &FileModel, n: Node) -> bool {
    named_children(n)
        .into_iter()
        .all(|pair| pair.kind() != "pair" || all_named_pure(fm, pair))
}

fn unary_pure(fm: &FileModel, n: Node) -> bool {
    n.child_by_field_name("operator")
        .map(|o| fm.text(o))
        .unwrap_or("")
        != "defined?"
        && all_named_pure(fm, n)
}

fn paren_pure(fm: &FileModel, n: Node) -> bool {
    let inner = named_children(n);
    inner.len() == 1 && pure(fm, inner[0])
}

const VETO_ANCESTORS: [&str; 14] = [
    "if",
    "unless",
    "if_modifier",
    "unless_modifier",
    "conditional",
    "while",
    "until",
    "while_modifier",
    "until_modifier",
    "for",
    "rescue",
    "rescue_modifier",
    "in_clause",
    "when",
];
const SCOPE_OWNERS: [&str; 8] = [
    "method",
    "singleton_method",
    "class",
    "module",
    "singleton_class",
    "block",
    "do_block",
    "lambda",
];

fn unconditionally_executed(write_node: Node) -> bool {
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if VETO_ANCESTORS.contains(&n.kind()) {
            return false;
        }
        if SCOPE_OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
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

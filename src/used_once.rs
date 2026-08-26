//! Variable-used-only-once detector: flags locals with exactly one plain
//! write and exactly one read where inlining is provably safe (pure RHS,
//! write dominates the read).

use std::collections::HashMap;

use tree_sitter::Node;

use crate::model::{Entry, FileModel, IntroKind, Read, Write, WriteKind};

#[derive(PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct UsedOnceOffense {
    pub line: usize,
    pub column: usize,
    pub name: String,
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

/// Straight-line execution check: no conditional/loop/rescue ancestor between
/// the write and its owning scope boundary.
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

/// Conservative RHS purity: literals, constants, self, and compositions of
/// comparisons/logical operators over those. Anything calling methods is out.
fn pure(fm: &FileModel, n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "true" | "false" | "nil" | "simple_symbol" | "symbol" | "self"
        | "constant" => true,
        "string" => string_without_interpolation(n),
        "array" | "range" | "binary" => all_named_pure(fm, n),
        "hash" => hash_pure(fm, n),
        "unary" => unary_pure(fm, n),
        // `(expr)` -- pure only when a single pure expression inside
        "parenthesized_statements" => paren_pure(fm, n),
        _ => false,
    }
}

fn named_children<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut cur = n.walk();
    n.children(&mut cur).filter(|c| c.is_named()).collect()
}

/// Every named child is pure.
fn all_named_pure(fm: &FileModel, n: Node) -> bool {
    named_children(n).into_iter().all(|c| pure(fm, c))
}

fn string_without_interpolation(n: Node) -> bool {
    let mut cur = n.walk();
    !n.children(&mut cur).any(|c| c.kind() == "interpolation")
        && n.child_by_field_name("interpolation").is_none()
}

/// Hash literal: both sides of every `pair` pure; punctuation-level named
/// nodes are ignored.
fn hash_pure(fm: &FileModel, n: Node) -> bool {
    named_children(n)
        .into_iter()
        .all(|pair| pair.kind() != "pair" || all_named_pure(fm, pair))
}

/// `defined?` results depend on scope state, so they are never pure.
fn unary_pure(fm: &FileModel, n: Node) -> bool {
    let op = n
        .child_by_field_name("operator")
        .map(|o| fm.text(o))
        .unwrap_or("");
    op != "defined?" && all_named_pure(fm, n)
}

fn paren_pure(fm: &FileModel, n: Node) -> bool {
    let inner = named_children(n);
    inner.len() == 1 && pure(fm, inner[0])
}

pub fn analyze(fm: &FileModel) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();

    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if let Some(offense) = single_use_offense(fm, &nodes, name, e) {
                out.push(offense);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

/// One plain write, one later read, pure RHS and straight-line execution:
/// the read can be inlined into the write.
fn single_use_offense<'t>(
    fm: &FileModel,
    nodes: &HashMap<usize, Node<'t>>,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    let w = exactly_one_plain_write(e)?;
    later_single_read(e, &w)?;
    let (rhs_node, write_node) = offense_nodes(nodes, &w)?;
    if !pure(fm, rhs_node) || !unconditionally_executed(write_node) {
        return None;
    }
    let (line, column) = fm.line_col(w.byte);
    Some(UsedOnceOffense {
        line,
        column,
        name: name.to_string(),
    })
}

/// Tree nodes for a write's RHS expression and for the write itself.
fn offense_nodes<'t>(
    nodes: &HashMap<usize, Node<'t>>,
    w: &Write,
) -> Option<(Node<'t>, Node<'t>)> {
    let (rhs_id, _) = w.rhs?;
    Some((*nodes.get(&rhs_id)?, *nodes.get(&w.node_id)?))
}

/// The entry is assign-introduced and has exactly one plain write.
fn exactly_one_plain_write(e: &Entry) -> Option<&Write> {
    if e.intro_kind != IntroKind::Assign || e.writes.len() != 1 {
        return None;
    }
    let w = &e.writes[0];
    (w.kind == WriteKind::Plain).then_some(w)
}

/// The single read happens after the write and outside any `defined?` guard.
fn later_single_read<'a>(e: &'a Entry, w: &Write) -> Option<&'a Read> {
    if e.reads.len() != 1 || e.reads[0].under_defined || e.reads[0].byte <= w.byte {
        return None;
    }
    Some(&e.reads[0])
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;

    fn flags(src: &str) -> Vec<UsedOnceOffense> {
        analyze(&model::build_from_str(src))
    }

    #[test]
    fn shorthand_hash_read_is_a_use() {
        let f = flags("def k\n  x = 42\n  g(x:)\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "x");
    }

    #[test]
    fn simple_single_use_is_flagged_at_write_line() {
        let f = flags("def k\n  tmp = 42\n  p tmp\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "tmp");
        assert_eq!(f[0].line, 2);
        assert_eq!(f[0].column, 2);
    }

    #[test]
    fn conditional_write_is_vetoed() {
        let f = flags("def k(c)\n  tmp = 42 if c\n  p tmp\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn impure_rhs_is_rejected() {
        let f = flags("def k(items)\n  tmp = items.size\n  p tmp\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn second_read_blocks_inlining() {
        let f = flags("def k\n  tmp = 42\n  p tmp\n  p tmp\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn defined_check_read_is_vetoed() {
        let f = flags("def k\n  tmp = 42\n  p defined?(tmp)\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn opassign_counts_as_second_write() {
        let f = flags("def k\n  tmp = 1\n  tmp += 2\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn rescue_variable_is_excluded() {
        let f = flags("def k\n  risky\nrescue => e\n  e.message\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn read_inside_later_block_still_inlinable() {
        let f = flags("def k(arr)\n  x = 42\n  arr.each { |i| p i + x }\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "x");
        assert_eq!(f[0].line, 2);
    }
}

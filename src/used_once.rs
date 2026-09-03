//! Variable-used-only-once detector: flags locals with exactly one plain
//! write and exactly one read where the RHS can be substituted at the read
//! (inlinable expression, write dominates the read).

use std::collections::HashMap;

use tree_sitter::Node;

use crate::inlinable::{immediate_substitutable, ruby_alias_stable, RUBY_IDENT, RUBY_UNITS};
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

/// RHS may move wholesale to the single read site. Literals/constants/self,
/// compositions of those, method call / index chains (including attached
/// blocks), and bare local reads all qualify.
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
    (0..n.named_child_count())
        .filter_map(|i| n.named_child(i as u32))
        .collect()
}

/// Every named child is pure.
fn all_named_pure(fm: &FileModel, n: Node) -> bool {
    named_children(n).into_iter().all(|c| pure(fm, c))
}

fn string_without_interpolation(n: Node) -> bool {
    !(0..n.named_child_count())
        .filter_map(|i| n.named_child(i as u32))
        .any(|c| c.kind() == "interpolation")
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
    n.child_by_field_name("operator")
        .map(|o| fm.text(o))
        .unwrap_or("") != "defined?"
        && all_named_pure(fm, n)
}

fn paren_pure(fm: &FileModel, n: Node) -> bool {
    let inner = named_children(n);
    inner.len() == 1 && pure(fm, inner[0])
}

/// A bare `tmp = source` alias is safe only when `source` is not written
/// between the alias assignment and its single read.
fn alias_source_stable(
    fm: &FileModel,
    scope: usize,
    name: &str,
    write_byte: usize,
    read_byte: usize,
) -> bool {
    ruby_alias_stable(fm, scope, name, write_byte, read_byte)
}

fn inlinable_rhs(
    fm: &FileModel,
    n: Node,
    scope: usize,
    write_byte: usize,
    read_byte: usize,
    write_site: Node,
) -> bool {
    if RUBY_UNITS.contains(&n.kind()) {
        return immediate_substitutable(write_site, read_byte);
    }
    if n.kind() == RUBY_IDENT {
        let name = fm.text(n);
        // Bare `foo` with no local binding is a vcall — same effect rules as `foo()`.
        if fm.lookup(scope, write_byte, name).is_none() {
            return immediate_substitutable(write_site, read_byte);
        }
        return alias_source_stable(fm, scope, name, write_byte, read_byte);
    }
    pure(fm, n)
}

pub fn analyze(fm: &FileModel) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();

    for (scope, scope_data) in fm.scopes.iter().enumerate() {
        for (name, e) in &scope_data.entries {
            if let Some(offense) = single_use_offense(fm, &nodes, scope, name, e) {
                out.push(offense);
            }
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

/// One plain write, one later read, inlinable RHS and straight-line execution:
/// the RHS can replace the read.
fn single_use_offense<'t>(
    fm: &FileModel,
    nodes: &HashMap<usize, Node<'t>>,
    scope: usize,
    name: &str,
    e: &Entry,
) -> Option<UsedOnceOffense> {
    let w = exactly_one_plain_write(e)?;
    let read = later_single_read(e, &w)?;
    let (rhs_node, write_node) = offense_nodes(nodes, &w)?;
    if !inlinable_rhs(fm, rhs_node, scope, w.byte, read.byte, write_node)
        || !unconditionally_executed(write_node) {
        return None;
    }
    Some(offense_at(fm, name, w.byte))
}

fn offense_at(fm: &FileModel, name: &str, byte: usize) -> UsedOnceOffense {
    UsedOnceOffense {
        line: fm.line_col(byte).0,
        column: fm.line_col(byte).1,
        name: name.to_string(),
    }
}

/// Tree nodes for a write's RHS expression and for the write itself.
fn offense_nodes<'t>(nodes: &HashMap<usize, Node<'t>>, w: &Write) -> Option<(Node<'t>, Node<'t>)> {
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
    fn shorthand_only_read_never_qualifies_as_inlinable() {
        // Inlining would demand the invalid `g(42:)`; the binding must stay.
        let f = flags("def k\n  x = 42\n  g(x:)\nend\n");
        assert!(f.is_empty(), "shorthand read cannot be inlined: {f:?}");
    }

    #[test]
    fn shorthand_read_on_one_binding_leaves_others_flagged() {
        // `a` is only read via shorthand -> stays; `b` has one plain read.
        let f = flags("def k\n  a = 5\n  b = 7\n  g(a:, b)\nend\n");
        assert_eq!(
            f,
            vec![UsedOnceOffense {
                line: 3,
                column: 2,
                name: "b".into()
            }]
        );
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
    fn method_call_rhs_is_flagged() {
        let f = flags("def k(items)\n  tmp = items.size\n  p tmp\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "tmp");
    }

    #[test]
    fn call_chain_with_block_is_flagged() {
        let f = flags(
            "def k(number)\n  matching_ids = pluck(:id).filter_map do |id|\n    id\n  end\n  where(id: matching_ids)\nend\n",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "matching_ids");
    }

    #[test]
    fn call_chain_in_scope_lambda_is_flagged() {
        let f = flags(
            "class K < ApplicationRecord\n  scope :for_shop, lambda { |shop_id|\n    matching_ids = pluck(:id).filter_map { |id| id }\n    where(id: matching_ids)\n  }\nend\n",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "matching_ids");
    }

    #[test]
    fn call_chain_rejected_with_intervening_statement() {
        let f = flags("def k\n  tmp = compute()\n  side_effect()\n  use(tmp)\nend\n");
        assert!(f.is_empty(), "effectful RHS must not cross statements: {f:?}");
    }

    #[test]
    fn vcall_rhs_rejected_with_intervening_statement() {
        // `compute` without () is an identifier vcall, not a local alias.
        let f = flags("def k\n  tmp = compute\n  side_effect()\n  use(tmp)\nend\n");
        assert!(f.is_empty(), "vcall RHS must not cross statements: {f:?}");
    }

    #[test]
    fn call_chain_in_block_read_rejected() {
        let f = flags("def k(arr)\n  tmp = compute()\n  arr.each { |i| p tmp }\nend\n");
        assert!(f.is_empty(), "read inside loop block must not inline calls: {f:?}");
    }

    #[test]
    fn call_in_modifier_value_rejected() {
        let f = flags("def k\n  tmp = compute()\n  return tmp if ok?\nend\n");
        assert!(
            f.is_empty(),
            "inlining into modifier value runs the call after the condition: {f:?}"
        );
    }

    #[test]
    fn call_in_conditional_body_rejected() {
        let f = flags("def k(c)\n  tmp = compute()\n  if c\n    use(tmp)\n  end\nend\n");
        assert!(f.is_empty(), "inlining into if body skips the call when false: {f:?}");
    }

    #[test]
    fn call_in_ternary_arm_rejected() {
        let f = flags("def k(c)\n  tmp = compute()\n  c ? use(tmp) : other\nend\n");
        assert!(f.is_empty(), "inlining into ternary arm is conditional: {f:?}");
    }

    #[test]
    fn call_in_condition_still_flagged() {
        // Condition always evaluates, so `if compute().empty?` matches the binding.
        let f = flags("def k\n  tmp = compute()\n  if tmp.empty?\n    bar\n  end\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "tmp");
    }

    #[test]
    fn call_in_modifier_condition_still_flagged() {
        let f = flags("def k\n  tmp = compute()\n  warn \"x\" unless tmp\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "tmp");
    }

    #[test]
    fn other_local_rhs_is_flagged() {
        let f = flags("def k(items)\n  tmp = items\n  p tmp\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "tmp");
    }

    #[test]
    fn alias_rejected_when_source_is_reassigned() {
        let f = flags("def k(items)\n  tmp = items\n  items = 1\n  p tmp\nend\n");
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

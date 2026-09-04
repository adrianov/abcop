//! Variable-used-only-once detector: flags locals with exactly one plain
//! write and exactly one read where the RHS can be substituted at the read
//! (inlinable expression, write dominates the read).

use std::collections::HashMap;

use tree_sitter::Node;

use crate::inlinable::ruby_inlinable_rhs;
use crate::model::{Entry, FileModel, IntroKind, Read, Write};

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
    let read = later_single_read(e, w)?;
    let (rhs_node, write_node) = offense_nodes(nodes, w)?;
    if !ruby_inlinable_rhs(fm, rhs_node, scope, w.byte, Some(read.byte), Some(write_node))
        || !w.unconditional
    {
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

/// Assign-introduced binding with exactly one plain write whose value is read.
/// Earlier unread overwrites do not block inlining the surviving write.
fn exactly_one_plain_write(e: &Entry) -> Option<&Write> {
    (e.intro_kind == IntroKind::Assign)
        .then(|| e.single_live_plain())
        .flatten()
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
    fn surviving_write_after_dead_overwrite_is_flagged() {
        let f = flags(
            "def k\n  tmp = create(:a)\n  tmp = 42\n  p tmp\nend\n",
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].name, "tmp");
        assert_eq!(f[0].line, 3);
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

    #[test]
    fn pure_ternary_with_interpolation_is_flagged() {
        let f = flags(
            "def k(frames)\n  note = frames > 1 ? \"x#{frames}\" : \"\"\n  p note\nend\n",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "note");
    }

    #[test]
    fn ternary_with_call_is_flagged_when_immediate() {
        let f = flags(
            "def k\n  hint = @hint.present? ? \"y#{@hint}\" : \"\"\n  p hint\nend\n",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "hint");
    }

    #[test]
    fn ternary_with_call_rejected_with_intervening_statement() {
        let f = flags(
            "def k\n  hint = @hint.present? ? \"y\" : \"\"\n  side_effect()\n  p hint\nend\n",
        );
        assert!(
            f.is_empty(),
            "effectful ternary must not cross statements: {f:?}"
        );
    }

    #[test]
    fn pure_ternary_may_cross_intervening_statement() {
        let f = flags(
            "def k(frames)\n  note = frames > 1 ? \"x#{frames}\" : \"\"\n  other = 1\n  \"#{note}#{other}\"\nend\n",
        );
        let names: Vec<_> = f.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"note"), "pure ternary can cross: {f:?}");
        assert!(names.contains(&"other"), "literal other still flagged: {f:?}");
    }

    #[test]
    fn vision_text_note_and_hint_are_flagged() {
        let f = flags(
            "def vision_text\n  frames = @images.size\n  note = frames > 1 ? \"\\n\\n# Photos\\n#{frames} photos\" : \"\"\n  hint = @hint.present? ? \"\\n\\n# Hint\\n#{@hint}\" : \"\"\n  \"#{PhotoMeal::Prompt.new.language_note}#{note}#{hint}\"\nend\n",
        );
        let names: Vec<_> = f.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"note"), "note missed: {f:?}");
        assert!(names.contains(&"hint"), "hint missed: {f:?}");
    }

    #[test]
    fn defined_expr_is_never_inlined() {
        let f = flags("def k\n  tmp = defined?(x)\n  p tmp\nend\n");
        assert!(f.is_empty(), "defined? must not be inlined: {f:?}");
    }

    #[test]
    fn hash_and_scope_resolution_are_flagged() {
        let f = flags(
            "def k(x)\n  h = { a: x }\n  c = Foo::Bar\n  p h\n  p c\nend\n",
        );
        let names: Vec<_> = f.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"h"), "hash missed: {f:?}");
        assert!(names.contains(&"c"), "scope_resolution missed: {f:?}");
    }
}

//! Variable-used-only-once detector: flags locals with exactly one plain
//! write and exactly one read where inlining is provably safe (pure RHS,
//! write dominates the read).

use std::collections::HashMap;

use tree_sitter::Node;

use crate::model::{FileModel, IntroKind, WriteKind};

#[derive(Debug)]
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
    "if", "unless", "if_modifier", "unless_modifier", "conditional", "while",
    "until", "while_modifier", "until_modifier", "for", "rescue",
    "rescue_modifier", "in_clause", "when",
];
const SCOPE_OWNERS: [&str; 8] = [
    "method", "singleton_method", "class", "module", "singleton_class",
    "block", "do_block", "lambda",
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
        "integer" | "float" | "true" | "false" | "nil" | "simple_symbol" | "symbol"
        | "self" | "constant" => true,
        "string" => {
            let mut cur = n.walk();
            !n.children(&mut cur)
                .any(|c| c.kind() == "interpolation")
                && n.child_by_field_name("interpolation").is_none()
        }
        "array" | "range" => {
            let mut cur = n.walk();
            n.children(&mut cur)
                .filter(|c| c.is_named())
                .all(|c| pure(fm, c))
        }
        "hash" => {
            let mut cur = n.walk();
            n.children(&mut cur)
                .filter(|c| c.is_named())
                .all(|pair| {
                    if pair.kind() != "pair" {
                        return true; // punctuation-level named nodes
                    }
                    let mut pc = pair.walk();
                    pair.children(&mut pc)
                        .filter(|c| c.is_named())
                        .all(|side| pure(fm, side))
                })
        }
        "binary" => {
            let mut cur = n.walk();
            n.children(&mut cur)
                .filter(|c| c.is_named())
                .all(|c| pure(fm, c))
        }
        "unary" => {
            let op = n
                .child_by_field_name("operator")
                .map(|o| fm.text(o))
                .unwrap_or("");
            op != "defined?"
                && {
                    let mut cur = n.walk();
                    n.children(&mut cur)
                        .filter(|c| c.is_named())
                        .all(|c| pure(fm, c))
                }
        }
        "parenthesized_statements" => {
            // `(expr)` — pure only when a single pure expression inside
            let mut cur = n.walk();
            let inner: Vec<_> = n
                .children(&mut cur)
                .filter(|c| c.is_named())
                .collect();
            inner.len() == 1 && pure(fm, inner[0])
        }
        _ => false,
    }
}

pub fn analyze(fm: &FileModel) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();

    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if e.intro_kind != IntroKind::Assign {
                continue;
            }
            if e.writes.len() != 1 || e.reads.len() != 1 {
                continue;
            }
            let w = e.writes[0];
            let r = e.reads[0];
            if w.kind != WriteKind::Plain || r.under_defined {
                continue;
            }
            if r.byte <= w.byte {
                continue;
            }
            let Some((rhs_id, _)) = w.rhs else { continue };
            let Some(&rhs_node) = nodes.get(&rhs_id) else {
                continue;
            };
            let Some(&write_node) = nodes.get(&w.node_id) else {
                continue;
            };
            if !pure(fm, rhs_node) {
                continue;
            }
            if !unconditionally_executed(write_node) {
                continue;
            }
            let (line, column) = fm.line_col(w.byte);
            out.push(UsedOnceOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;

    fn flags(src: &str) -> Vec<UsedOnceOffense> {
        analyze(&model::build_from_str(src))
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

//! RuboCop-compatible AbcSize calculator (default config):
//! sqrt(A²+B²+C²) over the body of every `def`/`defs` and
//! `define_method(:sym){}` block, post-order walk, with the unconditional
//! repeated-safe-navigation discount.
//!
//! Layout: [`calc`] owns the `Calc` accumulator and the unit walk,
//! [`count`] holds the per-node counters it dispatches to, [`helpers`]
//! the shared syntax predicates.
mod calc;
mod count;
mod flow;
mod helpers;

use tree_sitter::Node;

use crate::model::FileModel;

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct AbcOffense {
    pub line: usize,
    pub end_line: usize,
    pub column: usize,
    pub name: String,
    pub score: f64,
    pub vector: String,
}

/// Method and module ABC ceilings for one scan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub method: f64,
    pub module: f64,
}

pub(crate) fn fmt_vector(a: u32, b: u32, c: u32) -> String {
    format!("<{}, {}, {}>", a, b, c)
}

/// Position an AbcOffense at its unit root with rounded score and
/// vector -- the single RuboCop-compatible assembly point shared by
/// every language backend.
pub(crate) fn offense_at(unit: Node<'_>, name: &str, a: u32, b: u32, c: u32) -> AbcOffense {
    let raw = ((a * a + b * b + c * c) as f64).sqrt();
    let pos = unit.start_position();
    AbcOffense {
        line: pos.row + 1,
        end_line: unit.end_position().row + 1,
        column: pos.column,
        name: name.to_string(),
        score: (raw * 100.0).round() / 100.0,
        vector: fmt_vector(a, b, c),
    }
}

/// Parse an `"<A, B, C>"` metric vector into its three numbers; the
/// inverse of [`fmt_vector`].
pub(crate) fn parse_vector(vector: &str) -> (u32, u32, u32) {
    let nums = vector.trim_matches(|c| c == '<' || c == '>');
    let mut it = nums.split(", ");
    (
        it.next().unwrap().parse().unwrap(),
        it.next().unwrap().parse().unwrap(),
        it.next().unwrap().parse().unwrap(),
    )
}

/// Fitzpatrick module score: sum method vectors, then one magnitude.
pub(crate) fn module_score(scores: &[AbcOffense]) -> (u32, u32, u32, f64) {
    let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
    for o in scores {
        let (oa, ob, oc) = parse_vector(&o.vector);
        a += oa;
        b += ob;
        c += oc;
    }
    let raw = ((a * a + b * b + c * c) as f64).sqrt();
    (a, b, c, (raw * 100.0).round() / 100.0)
}

/// C `%g`-style formatting with 4 significant digits.
pub fn g4(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let exp = v.abs().log10().floor() as i32;
    if !(-4..4).contains(&exp) {
        return format!("{v:.3e}");
    }
    let prec = (3 - exp).clamp(0, 3) as usize;
    let s = format!("{v:.prec$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

pub fn all_scores(fm: &FileModel) -> Vec<AbcOffense> {
    let ctx = calc::build_ctx(fm);
    let mut offenses = Vec::new();
    calc::visit_units(fm, fm.tree.root_node(), &mut |unit, name| {
        if unit.child_by_field_name("body").is_some() {
            offenses.push(calc::score_unit(&ctx, fm, unit, name));
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;

    fn scores(src: &str) -> Vec<AbcOffense> {
        all_scores(&model::build_from_str(src))
    }

    #[test]
    fn compute_method_vector() {
        let s = scores(
            "def compute(items, factor)\n  total = 0\n  items.each_with_index do |item, i|\n    next if item.nil?\n    v = item * factor\n    total += v unless v < 10\n  end\n  total / factor\nend\n",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "compute");
        assert_eq!(s[0].vector, "<5, 4, 4>");
        assert!((s[0].score - 7.55).abs() < 1e-9);
    }

    #[test]
    fn comparisons_and_logical_ops_are_conditions_else_bonus() {
        let s = scores("def f(a)\n  if a == 1 && a < 5\n    :x\n  else\n    :y\n  end\nend\n");
        assert_eq!(s[0].vector, "<0, 0, 5>"); // if + else + == + && + <
        assert!((s[0].score - 5.0).abs() < 1e-9);
    }

    #[test]
    fn repeated_csend_on_same_local_discounted_until_reassigned() {
        let s = scores(
            "def g(x)\n  y = x&.to_s\n  z = x&.length\n  q = x&.size\n  y2 = x&.chars\nend\n",
        );
        assert_eq!(s[0].vector, "<4, 4, 1>"); // only first &. counts as condition
    }

    #[test]
    fn underscore_assignments_and_params_skipped_but_block_params_counted() {
        let s = scores("def h(items)\n  _tmp = items.map { |i| i }\n  items.length\nend\n");
        assert_eq!(s[0].vector, "<1, 2, 1>");
    }

    #[test]
    fn own_params_not_counted_nested_def_params_are() {
        let s = scores("def outer(a)\n  def inner(b) = b + 1\n  inner(a)\nend\n");
        assert_eq!(s.len(), 2);
        let outer = s.iter().find(|o| o.name == "outer").unwrap();
        assert_eq!(outer.vector, "<1, 2, 0>"); // b (nested param) + inner(a) + +
        let inner = s.iter().find(|o| o.name == "inner").unwrap();
        assert_eq!(inner.vector, "<0, 1, 0>");
    }

    #[test]
    fn iterating_block_pass_counts_as_condition() {
        let s = scores("def m(u)\n  u.map(&:to_s)\nend\n");
        // map call B=1; &:to_s under iterating method C=1
        assert_eq!(s[0].vector, "<0, 1, 1>");
    }

    #[test]
    fn non_iterating_block_not_a_condition() {
        let s = scores("def m(u)\n  u.transaction do |x|\n    x.commit\n  end\nend\n");
        // transaction call B=1; commit call B=1; block param x A=1
        assert_eq!(s[0].vector, "<1, 2, 0>");
    }

    #[test]
    fn masgn_targets_each_count_once() {
        let s = scores("def k(arr)\n  a, b = arr\n  a + b\nend\n");
        assert_eq!(s[0].vector, "<2, 1, 0>");
    }

    #[test]
    fn g4_matches_rubocop_significant_digits() {
        assert_eq!(g4(7.55), "7.55");
        assert_eq!(g4(17.0), "17");
        assert_eq!(g4(123.46), "123.5");
        assert_eq!(g4(0.5), "0.5");
        assert_eq!(g4(9.9999), "10");
    }

    #[test]
    fn define_method_uses_symbol_or_string_name_not_paren() {
        // Regression: `argument_list.child(0)` is the anonymous `(`.
        let s = scores(
            "define_method(:dyn) do |x|\n  x ? 1 : 0\nend\ndefine_method(\"str\") { |y| y ? 1 : 0 }\n",
        );
        let names: Vec<_> = s.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"dyn"), "got {names:?}");
        assert!(names.contains(&"str"), "got {names:?}");
        assert!(!names.iter().any(|n| n.contains('(')), "got {names:?}");
    }
}

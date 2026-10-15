//! RuboCop-compatible AbcSize calculator (default config):
//! sqrt(A²+B²+C²) over every `def`/`defs`, `define_method(:sym){}`
//! block, and any other block that is not already inside a method.
//! Post-order walk, with the unconditional repeated-safe-navigation
//! discount. Iteration blocks roll into the enclosing method or block.
//!
//! Layout: [`calc`] owns the `Calc` accumulator and the unit walk,
//! [`blocks`] decides which blocks are units, [`count`] holds the
//! per-node counters, [`helpers`] the shared syntax predicates.
mod blocks;
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
    /// Byte range of the unit in the source that was scored. Zero when the
    /// span is not a slice of that source (reconstructed template blocks).
    #[serde(default)]
    pub start_byte: usize,
    #[serde(default)]
    pub end_byte: usize,
    pub name: String,
    pub score: f64,
    pub vector: String,
    /// HEAD score when this hit is a rise. Absent for a new function and
    /// for `--full` / `--everything`, which report every over-ceiling method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<f64>,
}

/// One scored function, including those under the ceiling. The edit
/// filter pairs these; the reported `abc` list is only the hits.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Unit {
    pub name: String,
    pub line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub score: f64,
}

impl From<&AbcOffense> for Unit {
    fn from(o: &AbcOffense) -> Self {
        Self {
            name: o.name.clone(),
            line: o.line,
            end_line: o.end_line,
            start_byte: o.start_byte,
            end_byte: o.end_byte,
            score: o.score,
        }
    }
}

/// Method and module ABC ceilings for one scan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub method: f64,
    pub module: f64,
    /// When set, AbcSize and ModuleAbcSize keep a rise against HEAD, and
    /// UsedOnce / NeverUsed keep locals only inside functions whose text changed.
    pub increased: bool,
}

pub(crate) fn fmt_vector(a: u32, b: u32, c: u32) -> String {
    format!("<{}, {}, {}>", a, b, c)
}

/// Position an AbcOffense at its unit root with rounded score and
/// vector -- the single RuboCop-compatible assembly point shared by
/// every language backend.
pub(crate) fn offense_at(unit: Node<'_>, name: &str, a: u32, b: u32, c: u32) -> AbcOffense {
    let pos = unit.start_position();
    AbcOffense {
        line: pos.row + 1,
        end_line: unit.end_position().row + 1,
        column: pos.column,
        start_byte: unit.start_byte(),
        end_byte: unit.end_byte(),
        name: name.to_string(),
        score: score_of(a, b, c),
        vector: fmt_vector(a, b, c),
        prior: None,
    }
}

fn score_of(a: u32, b: u32, c: u32) -> f64 {
    (((a * a + b * b + c * c) as f64).sqrt() * 100.0).round() / 100.0
}

/// Lead clause of an AbcSize finding. A recorded HEAD score means the
/// working tree rose; otherwise the function is simply over the ceiling.
pub(crate) fn finding_clause(o: &AbcOffense) -> String {
    match o.prior {
        Some(was) => format!(
            "rose from {} to {} -- back to {} or below",
            g4(was),
            g4(o.score),
            g4(was)
        ),
        None => "is too high".to_string(),
    }
}

/// Parse an `"<A, B, C>"` metric vector into its three numbers; the
/// inverse of [`fmt_vector`].
pub(crate) fn parse_vector(vector: &str) -> (u32, u32, u32) {
    let mut it = vector.trim_matches(|c| c == '<' || c == '>').split(", ");
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

    (
        a,
        b,
        c,
        (((a * a + b * b + c * c) as f64).sqrt() * 100.0).round() / 100.0,
    )
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

        let names: Vec<_> = scores(
            "define_method(:dyn) do |x|\n  x ? 1 : 0\nend\ndefine_method(\"str\") { |y| y ? 1 : 0 }\n",
        )
        .into_iter()
        .map(|o| o.name)
        .collect();
        assert!(names.iter().any(|n| n == "dyn"), "got {names:?}");
        assert!(names.iter().any(|n| n == "str"), "got {names:?}");
        assert!(!names.iter().any(|n| n.contains('(')), "got {names:?}");
    }

    #[test]
    fn magic_constants_are_not_branches() {
        // RuboCop treats __FILE__/__LINE__/__ENCODING__ as constants, not sends.
        let s = scores("def f\n  __FILE__\n  __LINE__\n  __ENCODING__\n  bar\nend\n");
        assert_eq!(s[0].vector, "<0, 1, 0>");
    }

    #[test]
    fn signed_number_literals_are_not_branches() {
        // parser/RuboCop keeps signed numerics as literals; tree-sitter
        // emits unary + operand (integer/float/rational/complex).
        let s = scores("def f(x)\n  -1\n  +2.5\n  -1r\n  -1i\n  -x\n  !true\nend\n");
        assert_eq!(s[0].vector, "<0, 2, 0>"); // -x and !true only
    }

    #[test]
    fn block_outside_method_is_a_unit_and_method_vector_stays() {
        let s = scores(
            "def m(u)\n  u.transaction do |x|\n    x.commit\n  end\nend\nrun do\n  ready\nend\n",
        );
        let m = s.iter().find(|o| o.name == "m").unwrap();
        assert_eq!(m.vector, "<1, 2, 0>");
        let run = s.iter().find(|o| o.name == "run").unwrap();
        assert_eq!(run.vector, "<0, 1, 0>");
    }

    #[test]
    fn iterating_block_rolls_into_outer_block_but_toplevel_each_is_a_unit() {
        let names: Vec<_> = scores(
            "match do\n  list.each do |item|\n    item.values.map { |v| v * 2 }\n  end\nend\n",
        )
        .iter()
        .map(|o| o.name.clone())
        .collect::<Vec<_>>();
        assert_eq!(names, ["match"]);

        let top = scores("list.each do |item|\n  item.save\nend\n");
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].name, "each");
        assert_eq!(top[0].vector, "<0, 1, 0>");
    }

    #[test]
    fn define_method_keeps_inner_block_and_class_eval_skips_nested_def() {
        let defined = scores("define_method(:dyn) do |x|\n  tap do\n    y = 1\n  end\n  x\nend\n");
        assert_eq!(defined.len(), 1);
        assert_eq!(defined[0].name, "dyn");
        assert_eq!(defined[0].vector, "<1, 1, 0>");

        let eval = scores("class_eval do\n  def inner\n    total = 1\n    total\n  end\nend\n");
        let inner = eval.iter().find(|o| o.name == "inner").unwrap();
        assert_eq!(inner.vector, "<1, 0, 0>");
        let host = eval.iter().find(|o| o.name == "class_eval").unwrap();
        assert_eq!(host.vector, "<0, 0, 0>");
    }

    #[test]
    fn assigned_lambda_is_a_unit_and_lambda_inside_method_is_not() {
        let names: Vec<_> =
            scores("HANDLER = ->(x) { x ? 1 : 0 }\ndef m\n  y = ->(z) { z + 1 }\n  y\nend\n")
                .iter()
                .map(|o| o.name.clone())
                .collect::<Vec<_>>();
        assert_eq!(names, ["HANDLER", "m"]);
    }

    #[test]
    fn matcher_blocks_score_on_their_own() {
        assert_eq!(
            scores(
                "module TelemetryMetrics\n  extend self\n  def metric_points(name)\n    EXPORTER.pull\n    EXPORTER.metric_snapshots.select { it.name == name }.flat_map(&:data_points)\n  end\nend\nRSpec::Matchers.define :have_recorded do |value = nil, **attributes|\n  match do |subject|\n    args = recorded\n    expected = TelemetryMetrics.normalize_attributes(attributes)\n    if args.last.is_a?(Hash)\n      expected = TelemetryMetrics.normalize_attributes(args.last).merge(expected)\n      args = args[0...-1]\n    end\n    @expected = args.empty? ? [expected] : [args.first, expected]\n    @points = subject.is_a?(Array) ? subject : TelemetryMetrics.metric_points(TelemetryMetrics.instrument_name(subject))\n    matching = @points.select { TelemetryMetrics.normalize_attributes(it.attributes) == expected }\n    matching.any? && (args.empty? || matching.sum { TelemetryMetrics.point_value(it) } == args.first)\n  end\n  failure_message do |subject|\n    recorded_name(subject)\n  end\nend\n",
            )
            .into_iter()
            .map(|o| (o.name, o.vector))
            .collect::<Vec<_>>(),
            [
                ("metric_points".into(), "<0, 6, 3>".into()),
                ("have_recorded".into(), "<0, 2, 0>".into()),
                ("match".into(), "<7, 23, 9>".into()),
                ("failure_message".into(), "<0, 1, 0>".into()),
            ]
        );
    }
}

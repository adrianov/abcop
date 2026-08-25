//! RuboCop-compatible AbcSize calculator (default config):
//! sqrt(A²+B²+C²) over the body of every `def`/`defs` and
//! `define_method(:sym){}` block, post-order walk, with the unconditional
//! repeated-safe-navigation discount.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::model::FileModel;

pub(crate) const COMPARISON_OPS: &[&str] = &["==", "===", "!=", "<=", ">=", "<", ">"];

// Mirrors RuboCop Metrics::Utils::IteratingBlock::KNOWN_ITERATING_METHODS
static ITERATING: &[&str] = &[
    "all?", "any?", "bsearch", "bsearch_index", "chain", "chunk", "chunk_while",
    "collect", "collect!", "collect_concat", "combination", "count", "cycle",
    "d_permutation", "delete_if", "detect", "drop", "drop_while", "each",
    "each_cons", "each_entry", "each_index", "each_key", "each_pair", "each_slice",
    "each_value", "each_with_index", "each_with_object", "entries", "fetch",
    "fetch_values", "filter", "filter_map", "find", "find_all", "find_index",
    "flat_map", "grep", "grep_v", "group_by", "has_key?", "inject", "keep_if",
    "lazy", "map", "map!", "max", "max_by", "merge", "merge!", "min", "min_by",
    "minmax", "minmax_by", "none?", "one?", "partition", "permutation", "product",
    "reduce", "reject", "reject!", "repeat", "repeated_combination",
    "reverse_each", "select", "select!", "slice_after", "slice_before",
    "slice_when", "sort", "sort!", "sort_by", "sum", "take", "take_while",
    "tally", "to_h", "transform_keys", "transform_keys!", "transform_values",
    "transform_values!", "uniq", "with_index", "with_object", "zip",
];

#[derive(Debug)]
pub struct AbcOffense {
    pub line: usize,
    pub end_line: usize,
    pub column: usize,
    pub name: String,
    pub score: f64,
    pub vector: String,
}

pub(crate) fn iterating_call(fm: &FileModel, call: Node) -> bool {
    let Some(m) = call.child_by_field_name("method") else {
        return false;
    };
    {
        let name = fm.text(m);
        ITERATING.binary_search(&name).is_ok()
    }
}

/// `super` never counts; a `::`-qualified uppercase path is a constant hop.
pub(crate) fn is_non_send_callee(fm: &FileModel, call: Node, op: &str) -> bool {
    match call.child_by_field_name("method") {
        None => true,
        Some(m) => {
            let name = fm.text(m);
            name == "super"
                || (op == "::" && name.chars().next().is_some_and(|c| c.is_uppercase()))
        }
    }
}

fn param_target_name<'f>(fm: &'f FileModel, child: Node) -> Option<&'f str> {
    let target = child
        .child_by_field_name("name")
        .or_else(|| child.children(&mut child.walk()).find(|c| c.kind() == "identifier"))?;
    Some(fm.text(target))
}

pub(crate) fn param_names<'f>(fm: &'f FileModel, container: Node) -> Vec<&'f str> {
    let mut out = Vec::new();
    let mut cursor = container.walk();
    for child in container.children(&mut cursor) {
        match child.kind() {
            "identifier" => out.push(fm.text(child)),
            "optional_parameter"
            | "keyword_parameter"
            | "block_parameter"
            | "splat_parameter" => {
                if let Some(name) = param_target_name(fm, child) {
                    out.push(name);
                }
            }
            "destructured_parameter" => {
                out.extend(destructured_names(fm, child));
            }
            _ => {}
        }
    }
    out
}

fn destructured_names<'f>(fm: &'f FileModel, wrapper: Node) -> Vec<&'f str> {
    let mut sub = wrapper.walk();
    wrapper
        .children(&mut sub)
        .filter(|c| c.kind() == "identifier")
        .map(|c| fm.text(c))
        .collect()
}

fn define_method_argument(fm: &FileModel, call: Node) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let raw = fm.text(args.child(0)?);
    let name = raw.trim_start_matches(':').trim_matches(|c| c == '\'' || c == '"');
    (!name.is_empty()).then(|| name.to_string())
}

fn define_method_block_name(fm: &FileModel, block: Node) -> Option<String> {
    let call = block.parent()?;
    if call.kind() != "call" {
        return None;
    }
    let m = call.child_by_field_name("method")?;
    if fm.text(m) != "define_method" || call.child_by_field_name("receiver").is_some() {
        return None;
    }
    define_method_argument(fm, call)
}

/// Count assignment targets under a multiple-assignment left side.
pub(crate) fn masgn_target_count(fm: &FileModel, n: Node) -> u32 {
    let mut total = 0;
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                total += u32::from(!fm.text(child).starts_with('_'));
            }
            "instance_variable" | "class_variable" | "global_variable" | "constant" => {
                total += 1
            }
            "rest_assignment" | "destructured_left_assignment_list" => {
                total += masgn_target_count(fm, child)
            }
            _ => {}
        }
    }
    total
}

pub(crate) struct Calc<'f> {
    pub(crate) fm: &'f FileModel,
    pub(crate) csend_recv: &'f HashMap<usize, Box<str>>,
    pub(crate) vcall: &'f HashSet<usize>,
    pub(crate) seen_csend: HashSet<Box<str>>,
    pub(crate) a: u32,
    pub(crate) b: u32,
    pub(crate) c: u32,
}

impl<'f> Calc<'f> {
    pub(crate) const IS_ASSIGNMENT: [&'static str; 3] =
        ["assignment", "operator_assignment", "for"];
    pub(crate) const IS_FLOW: [&'static str; 14] = [
        "if", "unless", "elsif", "if_modifier", "unless_modifier", "conditional",
        "while", "until", "while_modifier", "until_modifier", "rescue",
        "rescue_modifier", "when", "in_clause",
    ];

    fn walk(&mut self, n: Node) {
        let children: Vec<_> = {
            let mut cursor = n.walk();
            n.children(&mut cursor).collect()
        };
        for ch in children {
            self.walk(ch);
        }
        self.count(n);
    }

    pub(crate) fn asgn_child_score(&self, ch: Node, is_lhs: bool) -> u32 {
        let k = ch.kind();
        let dispatch = (k == "call") || (k == "element_reference");
        let target = (k == "instance_variable")
            || (k == "class_variable")
            || (k == "global_variable")
            || (k == "constant");
        let counted = dispatch
            || (k == "identifier" && (is_lhs || self.vcall.contains(&ch.start_byte())))
            || (target && is_lhs);
        u32::from(counted)
    }
}

struct ScoreCtx {
    csend_recv: HashMap<usize, Box<str>>,
    vcall: HashSet<usize>,
}

fn score_unit(ctx: &ScoreCtx, fm: &FileModel, unit: Node, name: &str) -> AbcOffense {
    let mut calc = Calc {
        fm,
        csend_recv: &ctx.csend_recv,
        vcall: &ctx.vcall,
        seen_csend: HashSet::new(),
        a: 0,
        b: 0,
        c: 0,
    };
    if let Some(body) = unit.child_by_field_name("body") {
        calc.walk(body);
    }
    let raw = ((calc.a * calc.a + calc.b * calc.b + calc.c * calc.c) as f64).sqrt();
    let pos = unit.start_position();
    AbcOffense {
        line: pos.row + 1,
        end_line: unit.end_position().row + 1,
        column: pos.column,
        name: name.to_string(),
        score: (raw * 100.0).round() / 100.0,
        vector: fmt_vector(calc.a, calc.b, calc.c),
    }
}

fn visit_units(fm: &FileModel, n: Node, f: &mut impl FnMut(Node, &str)) {
    let is_fn = n.kind() == "method" || n.kind() == "singleton_method";
    if is_fn && let Some(name_node) = n.child_by_field_name("name") {
        f(n, fm.text(name_node));
    }
    let is_block = n.kind() == "block" || n.kind() == "do_block";
    if is_block && let Some(name) = define_method_block_name(fm, n) {
        f(n, &name);
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(fm, child, f);
    }
}

fn build_ctx(fm: &FileModel) -> ScoreCtx {
    ScoreCtx {
        csend_recv: fm
            .csend_sites
            .iter()
            .map(|(byte, name, _)| (*byte, name.clone()))
            .collect(),
        vcall: fm.vcall_sites.iter().copied().collect(),
    }
}

pub fn all_scores(fm: &FileModel) -> Vec<AbcOffense> {
    let ctx = build_ctx(fm);
    let mut offenses = Vec::new();
    visit_units(fm, fm.tree.root_node(), &mut |unit, name| {
        if unit.child_by_field_name("body").is_some() {
            offenses.push(score_unit(&ctx, fm, unit, name));
        }
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

pub fn analyze(fm: &FileModel, max: f64) -> Vec<AbcOffense> {
    all_scores(fm)
        .into_iter()
        .filter(|o| o.score > max)
        .collect()
}

pub(crate) fn fmt_vector(a: u32, b: u32, c: u32) -> String {
    format!("<{}, {}, {}>", a, b, c)
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
        let s = scores(
            "def f(a)\n  if a == 1 && a < 5\n    :x\n  else\n    :y\n  end\nend\n",
        );
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
}

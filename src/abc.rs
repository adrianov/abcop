//! RuboCop-compatible AbcSize calculator (default config):
//! sqrt(A²+B²+C²) over the body of every `def`/`defs` and
//! `define_method(:sym){}` block, post-order walk, with the unconditional
//! repeated-safe-navigation discount.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::model::FileModel;

const COMPARISON_OPS: &[&str] = &["==", "===", "!=", "<=", ">=", "<", ">"];

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
    pub column: usize,
    pub name: String,
    pub score: f64,
    pub vector: String,
}

struct Calc<'f> {
    fm: &'f FileModel,
    csend_recv: &'f HashMap<usize, Box<str>>,
    vcall: &'f HashSet<usize>,
    seen_csend: HashSet<Box<str>>,
    a: u32,
    b: u32,
    c: u32,
}

fn iterating_call(fm: &FileModel, call: Node) -> bool {
    let Some(m) = call.child_by_field_name("method") else {
        return false;
    };
    let name = fm.text(m);
    ITERATING.binary_search(&name).is_ok()
}

fn param_names<'f>(fm: &'f FileModel, container: Node) -> Vec<&'f str> {
    let mut out = Vec::new();
    let mut cursor = container.walk();
    for child in container.children(&mut cursor) {
        match child.kind() {
            "identifier" => out.push(fm.text(child)),
            "optional_parameter" | "keyword_parameter" | "block_parameter"
            | "splat_parameter" => {
                let target = child
                    .child_by_field_name("name")
                    .or_else(|| {
                        child
                            .children(&mut child.walk())
                            .find(|c| c.kind() == "identifier")
                    });
                if let Some(t) = target {
                    out.push(fm.text(t));
                }
            }
            "destructured_parameter" => {
                let mut sub = child.walk();
                for inner in child.children(&mut sub) {
                    if inner.kind() == "identifier" {
                        out.push(fm.text(inner));
                    }
                }
            }
            _ => {}
        }
    }
    out
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
    let args = call.child_by_field_name("arguments")?;
    let first = args.child(0)?;
    let raw = fm.text(first);
    let name = raw.trim_start_matches(':').trim_matches(|c| c == '\'' || c == '"');
    (!name.is_empty()).then(|| name.to_string())
}

/// Count assignment targets under a multiple-assignment left side.
fn masgn_target_count(fm: &FileModel, n: Node) -> u32 {
    let mut total = 0;
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = fm.text(child);
                if !name.starts_with('_') {
                    total += 1;
                }
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

impl<'f> Calc<'f> {
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

    fn asgn_child_score(&self, ch: Node, is_lhs: bool) -> u32 {
        let k = ch.kind();
        let dispatch = (k == "call") || (k == "element_reference");
        let ident = k == "identifier";
        let target = (k == "instance_variable")
            || (k == "class_variable")
            || (k == "global_variable")
            || (k == "constant");
        let counted =
            dispatch || (ident && (is_lhs || self.vcall.contains(&ch.start_byte()))) ||
                (target && is_lhs);
        u32::from(counted)
    }
    fn count(&mut self, n: Node) {
        // anonymous keyword tokens (`if`, `unless`, `when`, …) share kind
        // names with their clause nodes — only named nodes count
        if !n.is_named() {
            return;
        }
        let kind = n.kind();
        match kind {
            "assignment" => {
                let left = n.child_by_field_name("left");
                match left {
                    Some(l)
                        if l.kind() == "left_assignment_list"
                            || l.kind() == "right_assignment_list" =>
                    {
                        self.a += masgn_target_count(self.fm, l);
                    }
                    Some(l) if l.kind() == "identifier" => {
                        // lvasgn: resets the repeated-csend discount regardless
                        // of an underscore name (mirrors RuboCop ordering)
                        let name = self.fm.text(l);
                        self.seen_csend.remove(name);
                        if !name.starts_with('_') {
                            self.a += 1;
                        }
                    }
                    // setter forms (`obj.x =`, `h[k] =`): the child call /
                    // element_reference node contributes the parser :send
                    // branch when its own arm fires — assignment adds only A
                    _ => self.a += 1, // ivar/global/const targets
                }
            }
            "operator_assignment" => {
                // Mirrors compound_assignment: every DIRECT child that maps
                // to a parser dispatch/equals-assignment node and is not a
                // dotted setter contributes one assignment. `h[:k] += v`
                // counts (bracket setters have no operator loc), `o.a += v`
                // counts via its read-form lhs, and a call RHS counts too.
                let mut extra = 0;
                if let Some(l) = n.child_by_field_name("left") {
                    extra += self.asgn_child_score(l, true);
                }
                if let Some(r) = n.child_by_field_name("right") {
                    extra += self.asgn_child_score(r, false);
                }
                self.a += extra;
                let op = n
                    .child_by_field_name("operator")
                    .map(|o| self.fm.text(o))
                    .unwrap_or("");
                if op == "||=" || op == "&&=" {
                    self.c += 1;
                }
            }
            "for" => {
                self.a += 1;
                self.c += 1;
            }
            "if" | "unless" | "elsif" => {
                self.c += 1;
                let has_else_keyword = {
                    let mut cur = n.walk();
                    n.children(&mut cur).any(|ch| ch.kind() == "else")
                };
                if has_else_keyword {
                    self.c += 1;
                }
            }
            "if_modifier" | "unless_modifier" | "conditional" => self.c += 1,
            "while" | "until" | "while_modifier" | "until_modifier" => self.c += 1,
            "rescue" | "rescue_modifier" => {
                // a multi-clause rescue group is ONE :rescue node in the
                // parser AST; TS emits sibling clauses — count only the first
                let first_clause = n
                    .prev_named_sibling()
                    .map(|p| p.kind() != "rescue")
                    .unwrap_or(true);
                if kind == "rescue_modifier" || first_clause {
                    self.c += 1;
                }
                // `rescue => e` binds via lvasgn in parser AST → assignment
                if let Some(var) = n.child_by_field_name("variable") {
                    let named = var.children(&mut var.walk()).any(|c| {
                        c.kind() == "identifier" && !self.fm.text(c).starts_with('_')
                    });
                    if named {
                        self.a += 1;
                    }
                }
            }
            "when" | "in_clause" => self.c += 1,
            "binary" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|o| self.fm.text(o))
                    .unwrap_or("");
                if COMPARISON_OPS.contains(&op)
                    || matches!(op, "&&" | "||" | "and" | "or")
                {
                    self.c += 1;
                } else {
                    self.b += 1;
                }
            }
            // `element_reference` is parser-gem's `:send` aref → branch
            "element_reference" => self.b += 1,
            // `defined?` is a dedicated parser node type — neither branch
            // nor condition; its operand still walks normally below
            "unary" if n
                .child_by_field_name("operator")
                .map(|o| self.fm.text(o) == "defined?")
                .unwrap_or(false)
            => {}
            // `-1` / `+1.5` are folded into the integer literal by the parser
            "unary"
                if n
                    .child_by_field_name("operator")
                    .map(|o| matches!(self.fm.text(o), "-" | "+"))
                    .unwrap_or(false)
                    && n
                        .child_by_field_name("operand")
                        .map(|o| matches!(o.kind(), "integer" | "float"))
                        .unwrap_or(false)
                => {}
            "unary" => self.b += 1,
            "yield" => self.b += 1,
            // unresolved bare identifier == zero-arity method call → branch
            "identifier" => {
                if self.vcall.contains(&n.start_byte()) {
                    self.b += 1;
                }
            }
            "call" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|o| self.fm.text(o))
                    .unwrap_or("")
                    .to_string();
                if n.child_by_field_name("method")
                    .map(|m| {
                        let name = self.fm.text(m);
                        // `super` is never a send; `Rack::Utils` inside a
                        // call chain is a constant hop, not a send either
                        name == "super"
                            || (op == "::"
                                && name.chars().next().is_some_and(|c| c.is_uppercase()))
                    })
                    .unwrap_or(false)
                {
                    return;
                }
                match op.as_str() {
                    "" | "." => self.b += 1,
                    "&." => {
                        self.b += 1;
                        let discounted = n
                            .child_by_field_name("receiver")
                            .and_then(|r| self.csend_recv.get(&r.start_byte()))
                            .map(|name| !self.seen_csend.insert(name.clone()))
                            .unwrap_or(false);
                        if !discounted {
                            self.c += 1;
                        }
                    }
                    _ => self.b += 1,
                }
            }
            "block_argument" => {
                // `&:sym` / `&blk`: condition when the wrapping call iterates
                let arg_list = n.parent();
                let call = arg_list.and_then(|al| al.parent());
                if let Some(call) = call {
                    if call.kind() == "call" && iterating_call(self.fm, call) {
                        self.c += 1;
                    }
                }
            }
            "block" | "do_block" => {
                if let Some(p) = n.parent() {
                    if p.kind() == "call" && iterating_call(self.fm, p) {
                        self.c += 1;
                    }
                }
                if let Some(params) = n.child_by_field_name("parameters") {
                    self.a += param_names(self.fm, params)
                        .iter()
                        .filter(|nm| !nm.starts_with('_'))
                        .count() as u32;
                }
            }
            "lambda" => {
                if let Some(params) = n.child_by_field_name("parameters") {
                    self.a += param_names(self.fm, params)
                        .iter()
                        .filter(|nm| !nm.starts_with('_'))
                        .count() as u32;
                }
            }
            "method" | "singleton_method" => {
                // nested def params contribute to the enclosing unit
                if let Some(params) = n.child_by_field_name("parameters") {
                    self.a += param_names(self.fm, params)
                        .iter()
                        .filter(|nm| !nm.starts_with('_'))
                        .count() as u32;
                }
            }
            _ => {}
        }
    }
}

fn visit_units(fm: &FileModel, n: Node, f: &mut impl FnMut(Node, &str)) {
    match n.kind() {
        "method" | "singleton_method" => {
            if let Some(name_node) = n.child_by_field_name("name") {
                f(n, fm.text(name_node));
            }
        }
        "block" | "do_block" => {
            if let Some(name) = define_method_block_name(fm, n) {
                f(n, &name);
            }
        }
        _ => {}
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(fm, child, f);
    }
}

pub fn all_scores(fm: &FileModel) -> Vec<AbcOffense> {
    let csend_recv: HashMap<usize, Box<str>> = fm
        .csend_sites
        .iter()
        .map(|(byte, name, _)| (*byte, name.clone()))
        .collect();
    let vcall: HashSet<usize> = fm.vcall_sites.iter().copied().collect();

    let mut offenses = Vec::new();
    visit_units(fm, fm.tree.root_node(), &mut |unit, name| {
        let Some(body) = body_of_unit(unit) else {
            return;
        };
        let mut calc = Calc {
            fm,
            csend_recv: &csend_recv,
            vcall: &vcall,
            seen_csend: HashSet::new(),
            a: 0,
            b: 0,
            c: 0,
        };
        calc.walk(body);
        let raw =
            ((calc.a * calc.a + calc.b * calc.b + calc.c * calc.c) as f64).sqrt();
        let score = (raw * 100.0).round() / 100.0;
        let pos = unit.start_position();
        offenses.push(AbcOffense {
            line: pos.row + 1,
            column: pos.column,
            name: name.to_string(),
            score,
            vector: format!("<{}, {}, {}>", calc.a, calc.b, calc.c),
        });
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

fn body_of_unit(unit: Node) -> Option<Node> {
    unit.child_by_field_name("body").or_else(|| {
        let mut cursor = unit.walk();
        unit.children(&mut cursor)
            .find(|c| c.kind() == "body_statement" || c.kind() == "block_body")
    })
}

/// C `%g`-style formatting with 4 significant digits, matching RuboCop output.
pub fn g4(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let exp = v.abs().log10().floor() as i32;
    if !(-4..4).contains(&exp) {
        let s = format!("{v:.3e}");
        return s.replace("e0", "e+0");
    }
    let prec = (3 - exp).max(0) as usize;
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
        let src = "def g(x)\n  y = x&.to_s\n  z = x&.length\n  q = x&.size\n  y2 = x&.chars\nend\n";
        let s = scores(src);
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

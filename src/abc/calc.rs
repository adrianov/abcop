//! The `Calc` accumulator, its post-order walk, and the scoring loop
//! that turns each `def`/`defs`, `define_method`, and free-standing
//! block into an [`AbcOffense`].

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::blocks::{block_unit_name, define_method_block_name};
use super::{AbcOffense, offense_at};
use crate::model::FileModel;

pub(crate) struct Calc<'f> {
    pub(crate) fm: &'f FileModel<'f>,
    pub(crate) csend_recv: &'f HashMap<usize, Box<str>>,
    pub(crate) vcall: &'f HashSet<usize>,
    pub(crate) seen_csend: HashSet<Box<str>>,
    pub(crate) a: u32,
    pub(crate) b: u32,
    pub(crate) c: u32,
    /// Block units skip nested methods and nested block units so a
    /// module sum counts each body once. Methods and `define_method`
    /// keep the RuboCop walk, which still includes nested defs.
    skip_nested: bool,
}

impl<'f> Calc<'f> {
    pub(crate) const IS_ASSIGNMENT: [&'static str; 3] =
        ["assignment", "operator_assignment", "for"];
    pub(crate) const IS_FLOW: [&'static str; 14] = [
        "if",
        "unless",
        "elsif",
        "if_modifier",
        "unless_modifier",
        "conditional",
        "while",
        "until",
        "while_modifier",
        "until_modifier",
        "rescue",
        "rescue_modifier",
        "when",
        "in_clause",
    ];

    fn walk(&mut self, n: Node) {
        if self.skip_nested && hides_nested(self.fm, n) {
            return;
        }
        let children: Vec<_> = n.children(&mut n.walk()).collect();
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

pub(super) struct ScoreCtx {
    csend_recv: HashMap<usize, Box<str>>,
    vcall: HashSet<usize>,
}

pub(super) fn build_ctx(fm: &FileModel) -> ScoreCtx {
    ScoreCtx {
        csend_recv: fm
            .csend_sites
            .iter()
            .map(|(byte, name, _)| (*byte, name.clone()))
            .collect(),
        vcall: fm.vcall_sites.iter().copied().collect(),
    }
}

pub(crate) fn score_unit(ctx: &ScoreCtx, fm: &FileModel, unit: Node, name: &str) -> AbcOffense {
    let mut calc = Calc {
        fm,
        csend_recv: &ctx.csend_recv,
        vcall: &ctx.vcall,
        seen_csend: HashSet::new(),
        a: 0,
        b: 0,
        c: 0,
        skip_nested: skips_nested_units(fm, unit),
    };
    if let Some(body) = scored_body(unit) {
        calc.walk(body);
    }
    offense_at(unit, name, calc.a, calc.b, calc.c)
}

pub(super) fn visit_units(fm: &FileModel, n: Node, f: &mut impl FnMut(Node, &str)) {
    let is_fn = n.kind() == "method" || n.kind() == "singleton_method";
    if is_fn && let Some(name_node) = n.child_by_field_name("name") {
        f(n, fm.text(name_node));
    }
    if let Some(name) = block_unit_name(fm, n) {
        f(n, &name);
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(fm, child, f);
    }
}

/// A lambda scores like a method: its parameter list stays outside the body.
fn scored_body(unit: Node) -> Option<Node> {
    let body = unit.child_by_field_name("body")?;
    if unit.kind() == "lambda" {
        return body.child_by_field_name("body").or(Some(body));
    }
    Some(body)
}

fn skips_nested_units(fm: &FileModel, unit: Node) -> bool {
    matches!(unit.kind(), "block" | "do_block" | "lambda")
        && define_method_block_name(fm, unit).is_none()
}

fn hides_nested(fm: &FileModel, n: Node) -> bool {
    match n.kind() {
        "method" | "singleton_method" => true,
        "block" | "do_block" | "lambda" => block_unit_name(fm, n).is_some(),
        _ => false,
    }
}

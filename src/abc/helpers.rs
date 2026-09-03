//! Shared syntax predicates and tables used by the ABC counters.

use tree_sitter::Node;

use crate::model::FileModel;

pub(crate) const COMPARISON_OPS: &[&str] = &["==", "===", "!=", "<=", ">=", "<", ">"];

// Mirrors RuboCop Metrics::Utils::IteratingBlock::KNOWN_ITERATING_METHODS
static ITERATING: &[&str] = &[
    "all?",
    "any?",
    "bsearch",
    "bsearch_index",
    "chain",
    "chunk",
    "chunk_while",
    "collect",
    "collect!",
    "collect_concat",
    "combination",
    "count",
    "cycle",
    "d_permutation",
    "delete_if",
    "detect",
    "drop",
    "drop_while",
    "each",
    "each_cons",
    "each_entry",
    "each_index",
    "each_key",
    "each_pair",
    "each_slice",
    "each_value",
    "each_with_index",
    "each_with_object",
    "entries",
    "fetch",
    "fetch_values",
    "filter",
    "filter_map",
    "find",
    "find_all",
    "find_index",
    "flat_map",
    "grep",
    "grep_v",
    "group_by",
    "has_key?",
    "inject",
    "keep_if",
    "lazy",
    "map",
    "map!",
    "max",
    "max_by",
    "merge",
    "merge!",
    "min",
    "min_by",
    "minmax",
    "minmax_by",
    "none?",
    "one?",
    "partition",
    "permutation",
    "product",
    "reduce",
    "reject",
    "reject!",
    "repeat",
    "repeated_combination",
    "reverse_each",
    "select",
    "select!",
    "slice_after",
    "slice_before",
    "slice_when",
    "sort",
    "sort!",
    "sort_by",
    "sum",
    "take",
    "take_while",
    "tally",
    "to_h",
    "transform_keys",
    "transform_keys!",
    "transform_values",
    "transform_values!",
    "uniq",
    "with_index",
    "with_object",
    "zip",
];

pub(crate) fn iterating_call(fm: &FileModel, call: Node) -> bool {
    let Some(m) = call.child_by_field_name("method") else {
        return false;
    };
    ITERATING.binary_search(&fm.text(m)).is_ok()
}

/// `super` never counts; a `::`-qualified uppercase path is a constant hop.
pub(crate) fn is_non_send_callee(fm: &FileModel, call: Node, op: &str) -> bool {
    match call.child_by_field_name("method") {
        None => true,
        Some(m) => {
            let name = fm.text(m);
            name == "super" || (op == "::" && name.chars().next().is_some_and(|c| c.is_uppercase()))
        }
    }
}

fn param_target_name<'f>(fm: &'f FileModel, child: Node) -> Option<&'f str> {
    let target = child.child_by_field_name("name").or_else(|| {
        child
            .children(&mut child.walk())
            .find(|c| c.kind() == "identifier")
    })?;
    Some(fm.text(target))
}

pub(crate) fn param_names<'f>(fm: &'f FileModel, container: Node) -> Vec<&'f str> {
    let mut out = Vec::new();
    let mut cursor = container.walk();
    for child in container.children(&mut cursor) {
        match child.kind() {
            "identifier" => out.push(fm.text(child)),
            "optional_parameter" | "keyword_parameter" | "block_parameter" | "splat_parameter" => {
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
    wrapper
        .children(&mut wrapper.walk())
        .filter(|c| c.kind() == "identifier")
        .map(|c| fm.text(c))
        .collect()
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
            "instance_variable" | "class_variable" | "global_variable" | "constant" => total += 1,
            "rest_assignment" | "destructured_left_assignment_list" => {
                total += masgn_target_count(fm, child)
            }
            _ => {}
        }
    }
    total
}

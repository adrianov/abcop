//! Which Ruby blocks are AbcSize units, and what they are named.
//!
//! `define_method` stays a unit even inside a method (RuboCop). Every
//! other block is a unit only when it is not already inside a method or
//! `define_method`, and it is not an iteration block nested in another
//! block (`select` / `map` / `sum` roll into that block). A class-level
//! `each` has nowhere to roll, so it is a unit.

use tree_sitter::Node;

use super::helpers::iterating_call;
use crate::model::FileModel;

pub(super) fn block_unit_name(fm: &FileModel, n: Node) -> Option<String> {
    match n.kind() {
        "block" | "do_block" => named_block(fm, n),
        "lambda" => named_lambda(fm, n),
        _ => None,
    }
}

pub(super) fn define_method_block_name(fm: &FileModel, block: Node) -> Option<String> {
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

fn named_block(fm: &FileModel, block: Node) -> Option<String> {
    if let Some(name) = define_method_block_name(fm, block) {
        return Some(name);
    }
    if skip_block(fm, block) {
        return None;
    }
    Some(call_block_name(fm, block))
}

fn skip_block(fm: &FileModel, block: Node) -> bool {
    block.parent().is_some_and(|p| p.kind() == "lambda")
        || enclosed_by_method(fm, block)
        || rolls_into_block(fm, block)
}

fn named_lambda(fm: &FileModel, n: Node) -> Option<String> {
    if enclosed_by_method(fm, n) {
        return None;
    }
    Some(lambda_name(fm, n))
}

fn enclosed_by_method(fm: &FileModel, n: Node) -> bool {
    let mut parent = n.parent();
    while let Some(anc) = parent {
        if anc.kind() == "method" || anc.kind() == "singleton_method" {
            return true;
        }
        if matches!(anc.kind(), "block" | "do_block") && define_method_block_name(fm, anc).is_some()
        {
            return true;
        }
        parent = anc.parent();
    }
    false
}

fn rolls_into_block(fm: &FileModel, block: Node) -> bool {
    let Some(parent) = block.parent() else {
        return false;
    };
    let attached = parent.kind() == "call" || parent.kind() == "element_reference";
    attached && iterating_call(fm, parent) && ancestor_block(block)
}

fn ancestor_block(n: Node) -> bool {
    let mut parent = n.parent();
    while let Some(anc) = parent {
        if matches!(anc.kind(), "block" | "do_block" | "lambda") {
            return true;
        }
        parent = anc.parent();
    }
    false
}

fn call_block_name(fm: &FileModel, block: Node) -> String {
    let Some(call) = block.parent() else {
        return "block".into();
    };
    if call.kind() != "call" && call.kind() != "element_reference" {
        return "block".into();
    }
    if names_by_argument(fm, call)
        && let Some(name) = ident_argument(fm, call)
    {
        return name;
    }
    method_label(fm, call)
}

/// `define :have_recorded` and `define_method(:dyn)` are the block's name.
/// Other calls keep the method (`match`, `before`) so a hook argument
/// such as `:each` does not replace it.
fn names_by_argument(fm: &FileModel, call: Node) -> bool {
    call.child_by_field_name("method")
        .is_some_and(|m| matches!(fm.text(m), "define" | "define_method"))
}

fn method_label(fm: &FileModel, call: Node) -> String {
    call.child_by_field_name("method")
        .map(|m| fm.text(m).to_string())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "block".into())
}

fn ident_argument(fm: &FileModel, call: Node) -> Option<String> {
    let name = define_method_argument(fm, call)?;
    ident_like(&name).then_some(name)
}

fn ident_like(name: &str) -> bool {
    let mut chars = name
        .strip_suffix('?')
        .or_else(|| name.strip_suffix('!'))
        .unwrap_or(name)
        .chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn lambda_name(fm: &FileModel, n: Node) -> String {
    let Some(parent) = n.parent() else {
        return "lambda".into();
    };
    if !matches!(parent.kind(), "assignment" | "operator_assignment") {
        return "lambda".into();
    }
    parent
        .child_by_field_name("left")
        .filter(|left| {
            matches!(
                left.kind(),
                "identifier"
                    | "constant"
                    | "instance_variable"
                    | "class_variable"
                    | "global_variable"
            )
        })
        .map(|left| fm.text(left).to_string())
        .unwrap_or_else(|| "lambda".into())
}

fn define_method_argument(fm: &FileModel, call: Node) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    // Prefer named children: `child(0)` is the anonymous `(` of `argument_list`.
    let name = fm
        .text(args.named_child(0)?)
        .trim_start_matches(':')
        .trim_matches(|c| c == '\'' || c == '"');
    (!name.is_empty()).then(|| name.to_string())
}

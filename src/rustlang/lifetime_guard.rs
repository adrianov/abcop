//! Lifetime-guard veto for Rust UsedOnce (rustc E0716).

use tree_sitter::Node;

use super::scope::RustFile;

/// `let g = cell.borrow(); …` / `let g = cell.borrow().clone();` /
/// `let g = lock.lock().unwrap()` — any guard method in the RHS call chain
/// means the binding should stay (E0716 / double-borrow).
pub(super) fn guard_rhs_blocks(fm: &RustFile, rhs: Node) -> bool {
    chain_has_guard_method(fm, rhs)
}

fn chain_has_guard_method(fm: &RustFile, n: Node) -> bool {
    match n.kind() {
        "method_call_expression" => method_call_has_guard(fm, n),
        "call_expression" => call_expr_has_guard(fm, n),
        "await_expression" | "try_expression" | "parenthesized_expression" => n
            .named_child(0)
            .is_some_and(|c| chain_has_guard_method(fm, c)),
        _ => false,
    }
}

fn method_call_has_guard(fm: &RustFile, n: Node) -> bool {
    is_guard_producing_method(
        n.child_by_field_name("name")
            .map(|name| fm.text(name))
            .unwrap_or(""),
    ) || n
        .child_by_field_name("receiver")
        .is_some_and(|r| chain_has_guard_method(fm, r))
}

fn call_expr_has_guard(fm: &RustFile, n: Node) -> bool {
    let Some(func) = n.child_by_field_name("function") else {
        return false;
    };
    if func.kind() == "field_expression" {
        if is_guard_producing_method(
            func.child_by_field_name("field")
                .map(|f| fm.text(f))
                .unwrap_or(""),
        ) {
            return true;
        }
        return func
            .child_by_field_name("value")
            .is_some_and(|v| chain_has_guard_method(fm, v));
    }
    chain_has_guard_method(fm, func)
}

fn is_guard_producing_method(name: &str) -> bool {
    matches!(
        name,
        "borrow"
            | "borrow_mut"
            | "try_borrow"
            | "try_borrow_mut"
            | "lock"
            | "try_lock"
            | "read"
            | "write"
            | "try_read"
            | "try_write"
    )
}

/// `let n = path.file_name(); let s = n.to_string_lossy()` cannot be inlined:
/// the binding extends a call temporary so a borrow into a later `let` /
/// scrutinee stays valid (rustc E0716). Pure aliases (`let t = s`) are left
/// alone — those can still inline.
pub(super) fn lifetime_guard_blocks(fm: &RustFile, rhs: Node, read_byte: usize) -> bool {
    if !rhs_is_callish(rhs) {
        return false;
    }
    let Some(ident) = fm
        .tree
        .root_node()
        .descendant_for_byte_range(read_byte, read_byte)
    else {
        return false;
    };
    let Some(borrow_call) = borrow_method_on_ident(fm, ident) else {
        return false;
    };
    bound_borrow_result(borrow_call)
}

fn rhs_is_callish(n: Node) -> bool {
    match n.kind() {
        "call_expression" | "method_call_expression" | "await_expression" | "try_expression" => {
            true
        }
        "parenthesized_expression" => n.named_child(0).is_some_and(rhs_is_callish),
        _ => false,
    }
}

/// Sole read is `ident.borrow_method(…)` (possibly the start of a chain).
///
/// tree-sitter-rust often emits zero-arg methods as
/// `call_expression` + `field_expression` rather than `method_call_expression`.
fn borrow_method_on_ident<'t>(fm: &RustFile, ident: Node<'t>) -> Option<Node<'t>> {
    let parent = ident.parent()?;
    match parent.kind() {
        "method_call_expression" => method_call_borrow(fm, ident, parent),
        "field_expression" => field_call_borrow(fm, ident, parent),
        _ => None,
    }
}

fn method_call_borrow<'t>(fm: &RustFile, ident: Node<'t>, call: Node<'t>) -> Option<Node<'t>> {
    let receiver = call.child_by_field_name("receiver")?;
    if receiver.id() != ident.id() {
        return None;
    }
    let name = call.child_by_field_name("name")?;
    is_borrow_extending_method(fm.text(name)).then_some(call)
}

fn field_call_borrow<'t>(fm: &RustFile, ident: Node<'t>, field_expr: Node<'t>) -> Option<Node<'t>> {
    let value = field_expr.child_by_field_name("value")?;
    if value.id() != ident.id() {
        return None;
    }
    let field = field_expr.child_by_field_name("field")?;
    if !is_borrow_extending_method(fm.text(field)) {
        return None;
    }
    outer_call_or_field(field_expr)
}

/// Prefer the outer `foo.bar()` call node when the field is the call's function.
fn outer_call_or_field(field_expr: Node) -> Option<Node> {
    let Some(call) = field_expr.parent() else {
        return Some(field_expr);
    };
    if call.kind() != "call_expression" {
        return Some(field_expr);
    }
    let func = call.child_by_field_name("function")?;
    (func.id() == field_expr.id()).then_some(call)
}

/// Methods that typically return a borrow of `self` (or a guard over `self`).
fn is_borrow_extending_method(name: &str) -> bool {
    is_guard_producing_method(name)
        || matches!(
            name,
            "as_str"
                | "as_bytes"
                | "as_os_str"
                | "as_path"
                | "as_ref"
                | "as_mut"
                | "as_deref"
                | "as_deref_mut"
                | "as_slice"
                | "as_mut_slice"
                | "to_string_lossy"
                | "deref"
                | "deref_mut"
        )
}

/// True when a borrow-returning call (or chain from it) feeds a `let` /
/// `let else` / `if let` / `while let` / `match` binding.
fn bound_borrow_result(mut call: Node) -> bool {
    while let Some(parent) = call.parent() {
        match parent.kind() {
            "method_call_expression" | "field_expression" | "call_expression"
            | "try_expression" | "await_expression" | "reference_expression"
            | "parenthesized_expression" | "let_condition" | "condition" => {
                call = parent;
            }
            "let_declaration" => {
                return parent
                    .child_by_field_name("value")
                    .is_some_and(|v| node_contains(v, call));
            }
            "if_let_expression" | "while_let_expression" | "match_expression" => {
                return parent
                    .child_by_field_name("value")
                    .or_else(|| parent.child_by_field_name("condition"))
                    .is_some_and(|v| node_contains(v, call));
            }
            "else_clause" | "block" | "expression_statement" | "arguments" => return false,
            _ => return false,
        }
    }
    false
}

fn node_contains(outer: Node, inner: Node) -> bool {
    if outer.id() == inner.id() {
        return true;
    }
    let mut cur = inner.parent();
    while let Some(n) = cur {
        if n.id() == outer.id() {
            return true;
        }
        cur = n.parent();
    }
    false
}

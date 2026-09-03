//! Zig `variable_declaration` / assignment binding: the grammar aliases
//! both `const`/`var` heads and in-block assignment statements under one
//! kind, so splitting and walking live here for the collector and the
//! ABC tally to share.

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, dispatch};
use crate::scope_model::{IntroKind, Write};

use super::scope::Collector;

/// Both `const`/`var` declarations and the grammar's assignment-
/// statement alias share this kind.
pub(super) fn walk_var_decl(b: &mut Collector<'_>, n: Node, scope: usize) {
    let parts = split_var_decl(n, b.src);
    bind_decls(b, &parts, scope);
    rebind_assigns(b, &parts, scope);
    walk_left_operands(b, n, scope);
    if let Some(rhs) = parts.rhs {
        dispatch(b, rhs, scope);
    }
}

/// Plain `=` rebinds a visible local; compound operators rewrite-and-
/// read. Non-local targets (fields, derefs) keep operand reads only.
pub(super) fn walk_assignment(b: &mut Collector<'_>, n: Node, scope: usize) {
    let left = n.child_by_field_name("left");
    let right = n.child_by_field_name("right");
    let plain = assign_op_of(n, b.src) == Some("=");
    if let Some(left) = left {
        match plain_identifier(left) {
            Some(target) if b.rebind_local(target, scope, plain, right.map(|r| r.id())) => {}
            _ => b.walk_children(left, scope),
        }
    }
    if let Some(right) = right {
        dispatch(b, right, scope);
    }
}

/// How many A counts a `variable_declaration` contributes.
pub(super) fn var_decl_a_count(n: Node<'_>, src: &[u8]) -> u32 {
    let parts = split_var_decl(n, src);
    let named = parts
        .decls
        .iter()
        .chain(parts.assigns.iter().map(|(t, _)| t))
        .filter(|id| !ignored_name(**id, src))
        .count() as u32;
    if named > 0 {
        return named;
    }
    // Compound rewrite of a field/deref still counts as an assignment
    u32::from(assign_op_of(n, src).is_some_and(|op| op != "="))
}

pub(super) fn ignored_name(n: Node<'_>, src: &[u8]) -> bool {
    n.utf8_text(src)
        .map(|t| t.is_empty() || t.starts_with('_'))
        .unwrap_or(true)
}

struct DeclParts<'t> {
    decls: Vec<Node<'t>>,
    assigns: Vec<(Node<'t>, bool)>,
    rhs: Option<Node<'t>>,
}

fn bind_decls(b: &mut Collector<'_>, parts: &DeclParts<'_>, scope: usize) {
    let single = parts.decls.len() == 1 && parts.assigns.is_empty() && parts.rhs.is_some();
    for d in &parts.decls {
        let rhs_id = if single {
            parts.rhs.map(|r| r.id())
        } else {
            None
        };

        b.bind_var(
            *d,
            scope,
            Write::assign(d.start_byte(), d.id(), rhs_id),
            IntroKind::Assign,
        );
    }
}

fn rebind_assigns(b: &mut Collector<'_>, parts: &DeclParts<'_>, scope: usize) {
    for (target, plain) in &parts.assigns {
        if !b.rebind_local(*target, scope, *plain, parts.rhs.map(|r| r.id())) {
            b.model
                .record_read(scope, &b.text_of(*target).to_string(), target.start_byte());
        }
    }
}

/// Type annotations and field/deref left operands still produce reads;
/// declared/assigned identifiers are already consumed above.
fn walk_left_operands(b: &mut Collector<'_>, n: Node, scope: usize) {
    for child in pre_assign_nodes(n) {
        if matches!(child.kind(), "identifier" | "const" | "var") {
            continue;
        }
        if child.is_named() {
            dispatch(b, child, scope);
        }
    }
}

fn split_var_decl<'t>(n: Node<'t>, src: &'t [u8]) -> DeclParts<'t> {
    let mut decls = Vec::new();
    let mut assigns = Vec::new();
    let mut pending_decl = false;
    let mut saw_decl_kw = false;
    let plain = assign_op_of(n, src) == Some("=");
    for child in pre_assign_nodes(n) {
        match child.kind() {
            "const" | "var" => {
                pending_decl = true;
                saw_decl_kw = true;
            }
            "identifier" if pending_decl => {
                decls.push(child);
                pending_decl = false;
            }
            "identifier" if !saw_decl_kw => assigns.push((child, plain)),
            _ => pending_decl = false,
        }
    }
    DeclParts {
        decls,
        assigns,
        rhs: rhs_after_assign(n),
    }
}

fn pre_assign_nodes<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    for child in n.children(&mut n.walk()) {
        if is_assign_op_node(child) {
            break;
        }
        out.push(child);
    }
    out
}

fn rhs_after_assign(n: Node<'_>) -> Option<Node<'_>> {
    let mut past = false;
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        if !past {
            if is_assign_op_node(child) {
                past = true;
            }
            continue;
        }
        if child.is_named() && child.kind() != ";" {
            return Some(child);
        }
    }
    None
}

fn assign_op_of<'t>(n: Node<'t>, src: &'t [u8]) -> Option<&'t str> {
    if let Some(op) = n.child_by_field_name("operator") {
        return op.utf8_text(src).ok();
    }

    n.children(&mut n.walk())
        .find(|ch| is_assign_op_node(*ch))
        .and_then(|ch| ch.utf8_text(src).ok())
}

fn is_assign_op_node(n: Node<'_>) -> bool {
    !n.is_named() && ASSIGN_OPS.contains(&n.kind())
}

const ASSIGN_OPS: &[&str] = &[
    "=", "*=", "*%=", "*|=", "/=", "%=", "+=", "+%=", "+|=", "-=", "-%=", "-|=", "<<=", "<<|=",
    ">>=", "&=", "^=", "|=",
];

fn plain_identifier(n: Node<'_>) -> Option<Node<'_>> {
    (n.kind() == "identifier").then_some(n)
}

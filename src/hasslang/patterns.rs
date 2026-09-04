//! Pattern binder collection shared by AbcSize and the scope collector.

use tree_sitter::Node;

/// Identifiers a pattern binds (variables and as-pattern heads).
/// Constructors, literals and wildcards contribute nothing.
pub(super) fn pattern_vars<'t>(pattern: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    match pattern.kind() {
        "variable" => push_var(pattern, src, out),
        "as" => {
            if let Some(bind) = pattern.child_by_field_name("bind") {
                push_var(bind, src, out);
            }
            if let Some(inner) = pattern.child_by_field_name("pattern") {
                pattern_vars(inner, src, out);
            }
        }
        "wildcard" | "literal" | "constructor" | "comment" => {}
        _ => {
            let mut cursor = pattern.walk();
            for child in pattern.children(&mut cursor) {
                if child.is_named() {
                    pattern_vars(child, src, out);
                }
            }
        }
    }
}

pub(super) fn ignored_name(n: Node<'_>, src: &[u8]) -> bool {
    n.utf8_text(src)
        .map(|t| t.is_empty() || t.starts_with('_'))
        .unwrap_or(true)
}

fn push_var<'t>(n: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    if n.kind() == "variable" && !ignored_name(n, src) {
        out.push(n);
    }
}

/// How many A counts a pattern contributes.
pub(super) fn pattern_a_count(pattern: Node<'_>, src: &[u8]) -> u32 {
    let mut ids = Vec::new();
    pattern_vars(pattern, src, &mut ids);
    ids.len() as u32
}

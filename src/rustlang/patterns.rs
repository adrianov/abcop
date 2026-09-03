//! Pattern traversal: which identifiers a pattern binds.

use tree_sitter::Node;

use super::skip_subtree;

/// Lowercase, non-underscore names bind values; `_`-prefixed names and
/// enum constructors (uppercase first char) do not.
fn binds_value(name: &str) -> bool {
    !name.starts_with('_') && name.chars().next().is_some_and(|c| c.is_lowercase())
}

fn push_identifier<'t>(n: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    if binds_value(n.utf8_text(src).unwrap_or("")) {
        out.push(n);
    }
}

/// Identifiers bound by a let/for/if-let/match pattern.
pub(super) fn pattern_identifiers<'t>(pattern: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    // A bare identifier IS the whole pattern (let total = ...).
    if pattern.kind() == "identifier" {
        push_identifier(pattern, src, out);
        return;
    }
    if skippable_pattern(pattern.kind()) {
        return;
    }
    let mut cursor = pattern.walk();
    for child in pattern.children(&mut cursor) {
        descend_pattern(child, src, out);
    }
}

/// `_` wildcard and type/attribute territory never bind identifiers.
fn skippable_pattern(kind: &str) -> bool {
    kind == "_" || skip_subtree(kind)
}

/// One pattern child: bind bare identifiers, recurse into the rest.
fn descend_pattern<'t>(child: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    if child.kind() == "identifier" {
        push_identifier(child, src, out);
    } else if !skippable_pattern(child.kind()) {
        pattern_identifiers(child, src, out);
    }
}

/// Match-arm binders: identifiers before an optional `if` guard; after the
/// guard everything is a read.
pub(super) fn match_binders<'t>(pattern: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    let mut cursor = pattern.walk();
    for child in pattern.children(&mut cursor) {
        match child.kind() {
            "if" => return,
            "identifier" => {
                if binds_value(child.utf8_text(src).unwrap_or("")) {
                    out.push(child);
                }
            }
            "_" => {}
            k if skip_subtree(k) => {}
            _ => match_binders(child, src, out),
        }
    }
}

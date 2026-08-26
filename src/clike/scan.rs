//! Unit discovery and naming for the C-family backend: walking the shared
//! tree-sitter tree for named declarations (and name-bound anonymous
//! function-likes), plus locating each unit's body.

use std::collections::HashSet;

use tree_sitter::Node;

use super::spec::{Spec, node_text};

/// Display name of a unit root: `name` field, then a declarator chain
/// (C/C++), then the first identifier-shaped child before the body
/// (ObjC selectors), else `(anonymous)`.
pub(crate) fn declared_name(n: Node, src: &[u8]) -> String {
    if let Some(name) = n.child_by_field_name("name") {
        return node_text(name, src).to_string();
    }
    if let Some(name) = declarator_name(n, src) {
        return name;
    }
    selector_name(n, src).unwrap_or_else(|| "(anonymous)".to_string())
}

/// C/C++ `int (*f)(...)`-style chains: first identifier down the
/// `declarator` field spine.
fn declarator_name(n: Node, src: &[u8]) -> Option<String> {
    let mut cur = n.child_by_field_name("declarator");
    while let Some(c) = cur {
        match c.kind() {
            "identifier" | "field_identifier" | "property_identifier" => {
                return Some(node_text(c, src).to_string());
            }
            _ => cur = c.child_by_field_name("declarator"),
        }
    }
    None
}

/// ObjC: the selector parts precede the body; take the first one.
fn selector_name(n: Node, src: &[u8]) -> Option<String> {
    let mut cursor = n.walk();
    let mut stack: Vec<Node> = n.children(&mut cursor).collect();
    while let Some(c) = stack.pop() {
        if matches!(c.kind(), "body" | "compound_statement") {
            continue;
        }
        if matches!(
            c.kind(),
            "identifier" | "method_identifier" | "field_identifier"
        ) {
            return Some(node_text(c, src).to_string());
        }
        let mut inner = c.walk();
        stack.extend(c.children(&mut inner));
    }
    None
}

/// Name bound to an anonymous function-like by its parent, if any: the
/// `const f = () => {}` idiom.
fn anon_bound_name<'t>(n: Node<'t>, src: &'t [u8]) -> Option<String> {
    let p = n.parent()?;
    let (value_field, name_field) = match p.kind() {
        "variable_declarator" | "pair" | "property_definition" | "public_field_definition" => {
            ("value", "name")
        }
        "assignment_expression" => ("right", "left"),
        _ => return None,
    };
    if p.child_by_field_name(value_field)? != n {
        return None;
    }
    let key = p.child_by_field_name(name_field)?;
    Some(
        node_text(key, src)
            .trim_matches(|c: char| c == '\'' || c == '"')
            .to_string(),
    )
}

pub(crate) fn discover<'t>(
    spec: &Spec,
    n: Node<'t>,
    src: &[u8],
    out: &mut Vec<(Node<'t>, String)>,
    roots: &mut HashSet<usize>,
) {
    let kind = n.kind();
    if spec.units.contains(&kind) {
        out.push((n, declared_name(n, src)));
        roots.insert(n.start_byte());
    } else if spec.anon.contains(&kind)
        && let Some(name) = anon_bound_name(n, src)
    {
        out.push((n, name));
        roots.insert(n.start_byte());
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        discover(spec, child, src, out, roots);
    }
}

/// Unit body: the `body` field when the grammar has one (JS/TS/Swift,
/// C/C++ function_definition), else the first compound statement child
/// (ObjC method_definition carries no fields).
pub(crate) fn unit_body<'t>(n: Node<'t>) -> Option<Node<'t>> {
    if let Some(b) = n.child_by_field_name("body") {
        return Some(b);
    }
    let mut cursor = n.walk();
    n.children(&mut cursor)
        .find(|ch| ch.kind() == "compound_statement")
}

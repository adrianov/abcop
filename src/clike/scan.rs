//! Unit discovery and naming for the C-family backend: walking the shared
//! tree-sitter tree for named declarations (and name-bound anonymous
//! function-likes), plus locating each unit's body.

use std::collections::HashSet;

use tree_sitter::Node;

use super::spec::{Spec, node_text};

/// Display name of a unit root: `name` field, then a declarator chain
/// (C/C++), then the first identifier-shaped child before the body
/// (ObjC selectors), else `(anonymous)`.
///
/// When a `@declarator` is present but cannot be resolved, do **not**
/// fall through to the ObjC-style DFS: that walk picks the last parameter
/// name (`data`, `o`, …) for out-of-line C++ methods.
pub(crate) fn declared_name(n: Node, src: &[u8]) -> String {
    if let Some(name) = n.child_by_field_name("name") {
        return node_text(name, src).to_string();
    }
    if n.child_by_field_name("declarator").is_some() {
        return declarator_name(n, src).unwrap_or_else(|| "(anonymous)".to_string());
    }
    selector_name(n, src).unwrap_or_else(|| "(anonymous)".to_string())
}

/// C/C++ declarator chains: follow `@declarator` through pointer/array/
/// function wrappers; for `Class::method` take the qualified `@name`.
/// `reference_declarator` has no `@declarator` field — use its named child.
fn declarator_name(n: Node, src: &[u8]) -> Option<String> {
    let mut cur = n.child_by_field_name("declarator")?;
    loop {
        match cur.kind() {
            "identifier"
            | "field_identifier"
            | "property_identifier"
            | "destructor_name"
            | "operator_name"
            | "operator_cast" => {
                return Some(node_text(cur, src).to_string());
            }
            // Transfers::onFoo / ns::Foo<T>::bar → keep walking at `@name`
            "qualified_identifier" => {
                cur = cur.child_by_field_name("name")?;
            }
            _ => {
                if let Some(next) = cur.child_by_field_name("declarator") {
                    cur = next;
                } else if cur.kind().ends_with("_declarator") {
                    cur = cur.named_child(0)?;
                } else {
                    return None;
                }
            }
        }
    }
}

/// ObjC: build the selector from direct children of `method_definition`
/// (`pick:` / `foo:bar:`). Skip return `method_type` and do not enter
/// `method_parameter` identifiers — that wrongly picked `items` / `y`.
fn selector_name(n: Node, src: &[u8]) -> Option<String> {
    let mut name = String::new();
    for c in n.children(&mut n.walk()) {
        match c.kind() {
            "compound_statement" | "body" => break,
            "method_type" => {}
            "identifier" | "method_identifier" | "field_identifier" => {
                name.push_str(node_text(c, src));
            }
            "method_parameter" => name.push(':'),
            _ => {}
        }
    }
    (!name.is_empty()).then_some(name)
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

    n.children(&mut n.walk())
        .find(|ch| ch.kind() == "compound_statement")
}

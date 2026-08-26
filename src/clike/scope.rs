//! Collector walk for the JavaScript/TypeScript family on the shared
//! scope_model dispatcher.
//!
//! Grammar notes: member slots are `property_identifier` (a distinct
//! kind, so plain `identifier` reads never confuse them); loop heads
//! carry their protocol variable under `@left`; declarations bind via
//! `variable_declarator` (`@name` + initializer sibling). Plain `=`
//! rebinds a visible local; compound operators rewrite-and-read;
//! assignments to names no visible binding introduced create globals,
//! contributing operand reads only.

use tree_sitter::Node;

use crate::scope_model::walk::{dispatch, Backend, Spec};
use crate::scope_model::{IntroKind, Model, ScopeKind, Write};

// NOTE: JavaScript functions are closures -- nested functions read outer
// bindings freely -- so every scope opens as Block and resolution escapes
// to the root. Rust-style Function boundaries would sever cross-function
// reads, producing NeverUsed false positives on module-level state.
static SPEC: Spec = Spec {
    skip_kinds: &["import_statement"],
    block_scoped: &[
        "statement_block",
        "switch_body",
        "match_block",
        "function_declaration",
        "generator_function_declaration",
        "method_definition",
        "arrow_function",
        "function",
    ],
    function_kinds: &[],
    exclude_fields: &[],
};

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<crate::scope_model::Scope> {
    let mut c = Collector {
        src,
        model: Model::rooted(),
    };
    c.walk(root, 0);
    c.model.scopes
}

struct Collector<'a> {
    src: &'a [u8],
    model: Model,
}

impl Backend for Collector<'_> {
    fn spec(&self) -> &'static Spec {
        &SPEC
    }

    fn model(&mut self) -> &mut Model {
        &mut self.model
    }

    fn text_of(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn custom(&mut self, n: Node, scope: usize) {
        match n.kind() {
            // const/let/var declarators: a plain identifier name binds
            // with its initializer linked; destructuring patterns bind
            // every contained element name (no RHS link)
            "variable_declarator" => {
                match n.child_by_field_name("name").map(|x| x.kind()) {
                    Some("identifier") | None => {
                        if let Some(name) = n.child_by_field_name("name") {
                            let rhs = n.child_by_field_name("value").map(|v| v.id());
                            let w = Write::assign(name.start_byte(), name.id(), rhs);
                            self.bind_var(name, scope, w, IntroKind::Assign);
                            // initializer subtrees may contain closures
                            // reading outer bindings -- walk them too
                            if let Some(value) = n.child_by_field_name("value") {
                                dispatch(self, value, scope);
                            }
                        }
                    }
                    _ => {
                        if let Some(pattern) = n.child_by_field_name("name") {
                            self.bind_pattern_elements(pattern, scope);
                        }
                    }
                }
            }
            // shorthand object literals ({ diagLog }) read the identically
            // named local; keyed pairs fall through to generic walking
            "pair" if n.child_by_field_name("value").is_none() => {
                if let Some(key) = n.child_by_field_name("key") {
                    if matches!(
                        key.kind(),
                        "property_identifier" | "shorthand_property_identifier"
                    ) {
                        let name = self.text_of(key).to_string();
                        self.model.record_read(scope, &name, key.start_byte());
                    }
                }
            }
            "assignment_expression" | "augmented_assignment_expression" => {
                self.walk_assignment(n, scope);
            }
            // for-in / for-of heads are protocol
            k if k == "for_in_statement" || k == "for_of_statement" => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                self.walk_children_excluding_field(n, s, "left");
            }
            // ++/-- read and rewrite
            "update_expression" => {
                if let Some(operand) = n.named_child(0).filter(|o| o.kind() == "identifier") {
                    let w = Write::rewrite(operand.start_byte(), operand.id());
                    self.bind_var(operand, scope, w, IntroKind::Binding);
                    let name = self.text_of(operand).to_string();
                    self.model.record_read(scope, &name, operand.end_byte());
                } else {
                    let mut cursor = n.walk();
                    let children: Vec<_> = n.children(&mut cursor).collect();
                    for child in children {
                        dispatch(self, child, scope);
                    }
                }
            }
            _ => {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    dispatch(self, child, scope);
                }
            }
        }
    }
}

impl Collector<'_> {
    /// Bind every identifier introduced by a destructuring pattern.
    /// Array patterns contribute their elements; object patterns
    /// contribute shorthand keys and `key: target` values.
    fn bind_pattern_elements(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                let w = Write::assign(n.start_byte(), n.id(), None);
                self.bind_var(n, scope, w, IntroKind::Assign);
            }
            "rest_pattern" | "assignment_pattern" | "object_pattern" | "array_pattern" => {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    self.bind_pattern_elements(child, scope);
                }
            }
            "pair" => {
                if let Some(value) = n.child_by_field_name("value") {
                    self.bind_pattern_elements(value, scope);
                }
            }
            _ => {}
        }
    }

    /// Plain `=` rebinds a visible local; compound operators
    /// rewrite-and-read. Assignments to names no visible binding
    /// introduced create globals -- their operands are reads only.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain = n
            .child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
            == Some("=");
        if let Some(left) = left {
            if matches!(left.kind(), "array_pattern" | "object_pattern") {
                // destructuring reassignment: elements bind, occurrence
                // sites must not double-register as reads
                self.bind_pattern_elements(left, scope);
                if let Some(right) = right {
                    dispatch(self, right, scope);
                }
                return;
            }
            if left.kind() == "identifier" {
                let name = self.text_of(left).to_string();
                if self.model.lookup(scope, left.start_byte(), &name).is_some() {
                    let (w, intro) = if plain {
                        (
                            Write::assign(left.start_byte(), left.id(), right.map(|r| r.id())),
                            IntroKind::Assign,
                        )
                    } else {
                        (Write::rewrite(left.start_byte(), left.id()), IntroKind::Binding)
                    };
                    self.model.bind(scope, &name, w, intro);
                    if !plain {
                        self.model.record_read(scope, &name, left.end_byte());
                    }
                }
            } else {
                let mut cursor = left.walk();
                let children: Vec<_> = left.children(&mut cursor).collect();
                for child in children {
                    dispatch(self, child, scope);
                }
            }
        }
        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }

    fn walk(&mut self, n: Node, scope: usize) {
        dispatch(self, n, scope);
    }

    fn walk_children_excluding_field(&mut self, n: Node, scope: usize, field: &str) {
        let skipped = n.child_by_field_name(field).map(|c| c.id());
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            if Some(child.id()) != skipped {
                dispatch(self, child, scope);
            }
        }
    }
}

/// Conservative RHS purity for JavaScript/TypeScript: literals,
/// operator compositions, and template literals without substitutions.
/// References to other locals, calls, and member reads fail it.
pub(super) fn js_pure(n: Node) -> bool {
    match n.kind() {
        "number"
        | "string"
        | "template_string"
        | "true"
        | "false"
        | "null"
        | "undefined" => children_without_interpolation(n),
        "parenthesized_expression" | "binary_expression" | "unary_expression"
        | "typeof_expression" => children_pure(n),
        _ => false,
    }
}

fn children_without_interpolation(n: Node) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| ch.kind() != "substitution" && js_pure(ch))
}

fn children_pure(n: Node) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| js_pure(ch))
}

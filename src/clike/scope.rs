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

use crate::scope_model::walk::{Backend, Spec, dispatch};
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
    read_kinds: &["identifier"],
    exclude_fields: &[],
};

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<crate::scope_model::Scope> {
    let mut c = Collector {
        src,
        model: Model::rooted(),
    };
    dispatch(&mut c, root, 0);
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
            "variable_declarator" => self.bind_variable_declarator(n, scope),
            // shorthand object literals ({ diagLog }) read the identically
            // named local; keyed pairs fall through to generic walking
            "pair" if n.child_by_field_name("value").is_none() => self.read_shorthand_key(n, scope),
            "assignment_expression" | "augmented_assignment_expression" => {
                self.walk_assignment(n, scope);
            }
            // for-in / for-of heads are protocol
            k if k == "for_in_statement" || k == "for_of_statement" => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                self.walk_children_excluding_field(n, s, "left");
            }
            // ++/-- read and rewrite
            "update_expression" => self.bind_inc_dec(n, scope),
            _ => self.walk_children(n, scope),
        }
    }
}

impl Collector<'_> {
    /// Bind a const/let/var declarator: a plain identifier name binds with
    /// its initializer linked; destructuring patterns bind every contained
    /// element name (no RHS link). Initializer subtrees may contain closures
    /// reading outer bindings -- walked in the identifier case.
    fn bind_variable_declarator(&mut self, n: Node, scope: usize) {
        match n.child_by_field_name("name") {
            Some(name) if name.kind() == "identifier" => {
                self.bind_declarator_with_rhs_field(n, scope);
                if let Some(value) = n.child_by_field_name("value") {
                    dispatch(self, value, scope);
                }
            }
            Some(pattern) => self.bind_pattern_elements(pattern, scope),
            None => {}
        }
    }

    /// Shorthand object literal `{ diagLog }`: the key text is also a read
    /// of the identically named local.
    fn read_shorthand_key(&mut self, n: Node, scope: usize) {
        if let Some(key) = n.child_by_field_name("key")
            && matches!(
                key.kind(),
                "property_identifier" | "shorthand_property_identifier"
            )
        {
            let name = self.text_of(key).to_string();
            self.model.record_read(scope, &name, key.start_byte());
        }
    }

    /// ++/-- reads and rewrites the operand in place.
    fn bind_inc_dec(&mut self, n: Node, scope: usize) {
        if let Some(operand) = n.named_child(0).filter(|o| o.kind() == "identifier") {
            let w = Write::rewrite(operand.start_byte(), operand.id());
            self.bind_var(operand, scope, w, IntroKind::Binding);
            let name = self.text_of(operand).to_string();
            self.model.record_read(scope, &name, operand.end_byte());
        } else {
            self.walk_children(n, scope);
        }
    }

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

    /// Plain `=` rebinds a visible local; compound operators rewrite-and-read.
    /// Assignments to names with no visible local create globals -- their
    /// operands are reads only. Destructuring LHS rebinds pattern elements.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain = n
            .child_by_field_name("operator")
            .map_or(false, |o| self.text_of(o) == "=");
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
                self.rebind_local(left, scope, plain, right.map(|r| r.id()));
            } else {
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }
}

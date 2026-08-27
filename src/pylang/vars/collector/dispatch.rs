//! AST traversal driving [`super::Collector`]: top-level node-kind
//! dispatch plus the per-kind write/read handlers.

use tree_sitter::Node;

use super::super::{IntroKind, SKIP_KINDS, ScopeKind};
use super::Collector;

impl Collector<'_> {
    pub(super) fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        match kind {
            "function_definition" | "class_definition" | "lambda" => {
                self.nested_scope(n, scope, kind)
            }
            "assignment" | "augmented_assignment" | "named_expression" => self.write_head(n, scope),
            "as_pattern" | "case_clause" | "for_statement" | "for_in_clause" => {
                self.protocol_head(n, scope)
            }
            "keyword_argument" | "attribute" => self.operand_only(n, scope),
            "identifier" => self.read_identifier(n, scope),
            _ => self.walk_children(n, scope),
        }
    }

    /// Defs/classes open fresh Function/Class scopes (their `name` field
    /// belongs to the enclosing scope); lambda bodies roll up like Rust
    /// closures -- reads inside resolve outward through the Block scope.
    fn nested_scope(&mut self, n: Node, parent: usize, kind: &str) {
        let (kind, skip_field) = match kind {
            "function_definition" => (ScopeKind::Function, "name"),
            "class_definition" => (ScopeKind::Class, "name"),
            _ => (ScopeKind::Block, "parameters"),
        };
        let s = self.open_scope(kind, parent);
        self.walk_except(n, s, skip_field);
    }

    /// Assignment-family heads: identifiers written on the left,
    /// ordinary reads on the right.
    fn write_head(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "assignment" => self.walk_assignment(n, scope),
            "augmented_assignment" => self.walk_augmented(n, scope),
            _ => self.walk_named_expression(n, scope),
        }
    }

    /// Heads whose side bindings are not ordinary locals: loop targets
    /// are protocol bindings that never get tracked; `as` aliases and
    /// case patterns capture names separately.
    fn protocol_head(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "as_pattern" => self.walk_as_pattern(n, scope),
            "case_clause" => self.walk_case_clause(n, scope),
            // loop targets are protocol bindings, never tracked
            _ => self.walk_except(n, scope, "left"),
        }
    }

    /// Attribute reads and keyword-argument labels are structural: walk
    /// only the operand/value child, skipping the dotted/labelled name.
    fn operand_only(&mut self, n: Node, scope: usize) {
        let field = match n.kind() {
            "attribute" => "object",
            _ => "value",
        };
        if let Some(part) = n.child_by_field_name(field) {
            self.walk(part, scope);
        }
    }

    /// A bare identifier outside any write head is an ordinary read.
    fn read_identifier(&mut self, n: Node, scope: usize) {
        let name = self.text(n).to_string();
        self.record_read(scope, &name, n.start_byte());
    }

    pub(super) fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    /// Walk every child except the given field's subtree.
    fn walk_except(&mut self, n: Node, scope: usize, skip_field: &str) {
        let skipped = n.child_by_field_name(skip_field).map(|s| s.id());
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if Some(child.id()) == skipped {
                continue;
            }
            self.walk(child, scope);
        }
    }

    /// `x = rhs`: a candidate write on the left, RHS walked for reads.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        if let Some(left) = left {
            if left.kind() == "identifier" {
                let rhs = right.map(|r| r.id());
                self.bind_name(left, scope, IntroKind::Assign, rhs);
            } else {
                self.bind_targets(left, scope);
            }
        }
        if let Some(right) = right {
            self.walk(right, scope);
        }
    }

    /// Reads the previous value and rewrites: neither candidate.
    /// Reference targets (`obj.x += 1`) contribute operand reads only.
    fn walk_augmented(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        if let Some(left) = left {
            if left.kind() == "identifier" {
                let name = self.text(left).to_string();
                self.bind_name(left, scope, IntroKind::Binding, None);
                self.record_read(scope, &name, left.start_byte() + 1);
            } else {
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = right {
            self.walk(right, scope);
        }
    }

    /// `(x := value)`: candidate write on the name, value walked.
    fn walk_named_expression(&mut self, n: Node, scope: usize) {
        if let (Some(name_node), Some(value)) = (
            n.child_by_field_name("name"),
            n.child_by_field_name("value"),
        ) {
            self.bind_name(name_node, scope, IntroKind::Assign, Some(value.id()));
            self.walk(value, scope);
        }
    }

    /// Value walks normally; everything after the `as` token is the
    /// alias binding.
    fn walk_as_pattern(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        let mut after_as = false;
        for child in children {
            if child.kind() == "as" {
                after_as = true;
            } else if after_as {
                self.bind_alias(child, scope);
            } else {
                self.walk(child, scope);
            }
        }
    }

    /// Pattern captures bind names; guard and body are code.
    fn walk_case_clause(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            if child.kind() == "case_pattern" {
                self.bind_captures(child, scope);
            } else {
                self.walk(child, scope);
            }
        }
    }
}

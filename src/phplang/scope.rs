//! Collector walk for PHP: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! PHP notes: variable names carry `$` in the tree and are stored and
//! reported without it; braces are NOT scopes (function-flat model), so
//! only capturing closures (`anonymous_function`, `arrow_function`) get
//! a Block scope; foreach heads are pure protocol.

use tree_sitter::Node;

use crate::scope_model::{IntroKind, Model, Scope, ScopeKind, Write, child_of_kind};

/// Subtrees carrying no local-variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "formal_parameters",
    "property_declaration",
    "const_declaration",
    "namespace_definition",
    "namespace_use_declaration",
];

/// Kinds that open a nested scope: only capturing closures.
const BLOCK_SCOPED: &[&str] = &["anonymous_function", "arrow_function"];

pub(super) fn collect(root: Node, src: &[u8]) -> Vec<Scope> {
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

impl Collector<'_> {
    /// PHP variable names keep `$` in the tree; store/report without it.
    fn var_name(&self, n: Node) -> String {
        let raw = n.utf8_text(self.src).unwrap_or("");
        raw.strip_prefix('$').unwrap_or(raw).to_string()
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn bind_var(&mut self, name_node: Node, scope: usize, w: Write, intro: IntroKind) {
        let name = self.var_name(name_node);
        self.model.bind(scope, &name, w, intro);
    }

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        match kind {
            k if BLOCK_SCOPED.contains(&k) => self.open_block_scope(n, scope),
            "function_definition" | "method_declaration" => self.open_function_scope(n, scope),
            "assignment_expression" => self.walk_assignment(n, scope),
            "augmented_assignment_expression" => self.walk_compound_assignment(n, scope),
            "foreach_statement" => self.walk_foreach_head(n, scope),
            "catch_clause" => self.walk_catch_clause(n, scope),
            "variable_name" => self.read_variable(n, scope),
            _ => self.walk_children(n, scope),
        }
    }

    /// Only capturing closures open a nested block scope.
    fn open_block_scope(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        self.walk_children(n, s);
    }

    fn open_function_scope(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Function, scope);
        self.walk_children(n, s);
    }

    /// Walks children except `excluded`, in the given scope; used where
    /// a head/binder node is handled separately from the subtree walk.
    fn walk_children_excluding(&mut self, n: Node, excluded: Option<usize>, scope: usize) {
        let mut cursor = n.walk();
        n.children(&mut cursor)
            .filter(|c| Some(c.id()) != excluded)
            .for_each(|c| self.walk(c, scope));
    }

    /// the `as` head is loop protocol -- never tracked, like
    /// Python for-targets and Go range heads
    fn walk_foreach_head(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        let skipped = child_of_kind(n, "pair").map(|p| p.id());
        self.walk_children_excluding(n, skipped, s);
    }

    /// `catch (E $e)`: bind the first variable_name, but exclude it from
    /// the walk -- its own occurrence must never register as a read
    fn walk_catch_clause(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
        let binder = child_of_kind(n, "variable_name");
        let skipped = binder.as_ref().map(|b| b.id());
        self.walk_children_excluding(n, skipped, s);
        if let Some(b) = binder {
            let w = Write::rewrite(b.start_byte(), b.id());
            self.bind_var(b, s, w, IntroKind::Binding);
        }
    }

    fn read_variable(&mut self, n: Node, scope: usize) {
        let name = self.var_name(n);
        if name == "this" {
            return;
        }
        self.model.record_read(scope, &name, n.start_byte());
    }

    /// Plain `=` rebinds variables (destructuring lists expand per
    /// element); member/subscript targets contribute operand reads only.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        if let Some(left) = left {
            self.bind_target(left, right.map(|r| r.id()), scope);
        }
        if let Some(right) = right {
            self.walk(right, scope);
        }
    }

    /// How a plain `=` target binds: a direct variable rebinds (carrying
    /// the value node id), a destructuring list expands per element, and
    /// member/subscript targets contribute operand reads only.
    fn bind_target(&mut self, target: Node, value_id: Option<usize>, scope: usize) {
        match target.kind() {
            "variable_name" => {
                let w = Write::assign(target.start_byte(), target.id(), value_id);
                self.bind_var(target, scope, w, IntroKind::Assign);
            }
            "list_literal" => self.expand_list(target, scope),
            _ => self.walk_children(target, scope),
        }
    }

    /// [$a, $b] = ... : each element binds per name
    fn expand_list(&mut self, list: Node, scope: usize) {
        let mut c = list.walk();
        for el in list.children(&mut c) {
            match el.kind() {
                "variable_name" => {
                    let w = Write::assign(el.start_byte(), el.id(), None);
                    self.bind_var(el, scope, w, IntroKind::Assign);
                }
                "," | "[" | "]" => {}
                _ => self.walk(el, scope),
            }
        }
    }

    /// Compound assignment reads the previous value and rewrites.
    fn walk_compound_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        if let Some(left) = left {
            if left.kind() == "variable_name" {
                let byte = left.start_byte();
                let w = Write::rewrite(byte, left.id());
                let name = self.var_name(left);
                self.model.bind(scope, &name, w, IntroKind::Binding);
                self.model.record_read(scope, &name, byte + 1);
            } else {
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = n.child_by_field_name("right") {
            self.walk(right, scope);
        }
    }
}

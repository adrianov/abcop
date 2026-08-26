//! Collector walk for Java: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! Java notes: member name slots (`field_access`'s `@field`,
//! `method_invocation`'s `@name`) and package-qualified type names are
//! never variable reads; enhanced-for heads and catch/resource bindings
//! are protocol, never candidates; braces inside `switch` groups share
//! the enclosing function scope via Block resolution.

use tree_sitter::Node;

use crate::scope_model::{IntroKind, Model, Scope, ScopeKind, Write, child_of_kind};

/// Subtrees carrying no local-variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "formal_parameters",
    "field_declaration",
    "import_declaration",
    "package_declaration",
    "scoped_identifier",
];

/// Expressions whose given named fields are member references, not
/// variables: walking skips exactly those slots.
const EXCLUDE_FIELDS: &[(&str, &str)] = &[
    ("method_invocation", "name"),
    ("field_access", "field"),
    ("nullsafe_member_call_expression", "name"),
];

/// Kinds that open a nested scope: blocks, lambdas and switch blocks
/// capture outward; everything else stops resolution.
const BLOCK_SCOPED: &[&str] = &["block", "lambda_expression", "switch_block"];

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
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn bind_var(&mut self, name_node: Node, scope: usize, w: Write, intro: IntroKind) {
        let name = self.text(name_node).to_string();
        self.model.bind(scope, &name, w, intro);
    }

    /// Bind a `variable_declarator`'s `@name`, optionally linking its
    /// `@value` as the inlinable RHS.
    fn bind_declarator(&mut self, n: Node, scope: usize, allow_rhs: bool) {
        if let Some(name) = n.child_by_field_name("name")
            && name.kind() == "identifier"
        {
            let rhs = if allow_rhs {
                n.child_by_field_name("value").map(|v| v.id())
            } else {
                None
            };
            let w = Write::assign(name.start_byte(), name.id(), rhs);
            self.bind_var(name, scope, w, IntroKind::Assign);
        }
    }

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();
        if SKIP_KINDS.contains(&kind) {
            return;
        }
        if let Some((_, field)) = EXCLUDE_FIELDS.iter().find(|(k, _)| *k == kind)
            && let Some(excluded) = n.child_by_field_name(field).map(|c| c.id())
        {
            let mut cursor = n.walk();
            n.children(&mut cursor)
                .filter(|c| c.id() != excluded)
                .for_each(|c| self.walk(c, scope));
            return;
        }
        if BLOCK_SCOPED.contains(&kind) {
            let s = self.model.open_scope(ScopeKind::Block, scope);
            self.walk_children(n, s);
            return;
        }
        match kind {
            "method_declaration" | "constructor_declaration" => {
                let s = self.model.open_scope(ScopeKind::Function, scope);
                self.walk_children(n, s);
            }
            "local_variable_declaration" => {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    match child.kind() {
                        "variable_declarator" => self.bind_declarator(child, scope, true),
                        _ => self.walk(child, scope),
                    }
                }
            }
            "assignment_expression" | "augmented_assignment_expression" => {
                self.walk_assignment(n, scope);
            }
            "for_statement" => {
                // head declarations bind protocol variables so body reads
                // resolve locally; condition and updates walk normally
                let s = self.model.open_scope(ScopeKind::Block, scope);
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    match child.kind() {
                        "local_variable_declaration" => child
                            .children(&mut child.walk())
                            .filter(|d| d.kind() == "variable_declarator")
                            .for_each(|d| self.bind_declarator(d, s, false)),
                        _ => self.walk(child, s),
                    }
                }
            }
            "catch_clause" => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                // bind the parameter's name, but exclude the whole
                // parameter subtree: its variable_name must never leak
                // into the read path
                let param = child_of_kind(n, "catch_formal_parameter");
                if let Some(bp) = param
                    && let Some(name) = bp.child_by_field_name("name")
                {
                    let w = Write::rewrite(name.start_byte(), name.id());
                    self.bind_var(name, s, w, IntroKind::Binding);
                }
                let skipped = param.map(|p| p.id());
                let mut cursor = n.walk();
                n.children(&mut cursor)
                    .filter(|c| Some(c.id()) != skipped)
                    .for_each(|c| self.walk(c, s));
            }
            "resource" => {
                // try-with-resources binding; its initializer still reads
                if let Some(name) = n
                    .child_by_field_name("name")
                    .filter(|n| n.kind() == "identifier")
                {
                    let w = Write::rewrite(name.start_byte(), name.id());
                    self.bind_var(name, scope, w, IntroKind::Binding);
                }
                if let Some(value) = n.child_by_field_name("value") {
                    self.walk(value, scope);
                }
            }
            "identifier" => {
                let name = self.text(n).to_string();
                self.model.record_read(scope, &name, n.start_byte());
            }
            _ => self.walk_children(n, scope),
        }
    }

    /// Assignments rebind variables -- plain `=` records one candidate
    /// write per identifier target, compound operators rewrite-and-read;
    /// field/array targets contribute operand reads only.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let plain = n
            .child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
            == Some("=");
        if let Some(left) = left {
            if left.kind() == "identifier" {
                let name = self.text(left).to_string();
                let w = if plain {
                    Write::assign(
                        left.start_byte(),
                        left.id(),
                        n.child_by_field_name("right").map(|r| r.id()),
                    )
                } else {
                    Write::rewrite(left.start_byte(), left.id())
                };
                let intro = if plain {
                    IntroKind::Assign
                } else {
                    IntroKind::Binding
                };
                self.model.bind(scope, &name, w, intro);
                if !plain {
                    self.model.record_read(scope, &name, left.end_byte());
                }
            } else {
                self.walk_children(left, scope);
            }
        }
        // the left side was handled above; walk the remaining children
        self.walk_children(n, scope);
    }
}

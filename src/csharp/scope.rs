//! Collector walk for C#: which nodes open scopes, which bind or read
//! variables. Evaluation lives in [`crate::scope_model`].
//!
//! C# notes: member name slots (`member_access_expression`'s `@name`)
//! and qualified/type names are never variable reads; foreach heads and
//! catch declarations are protocol; `out var`/declaration expressions
//! bind as rewrites (they read through the `out`).

use tree_sitter::Node;

use crate::scope_model::{child_of_kind, IntroKind, Model, Scope, ScopeKind, Write};

/// Subtrees carrying no local-variable writes or reads.
const SKIP_KINDS: &[&str] = &[
    "parameter_list",
    "field_declaration",
    "using_directive",
    "namespace_declaration",
    "file_scoped_namespace_declaration",
    "qualified_name",
];

/// Kinds that open a nested scope: blocks, lambdas, anonymous methods
/// and switch bodies capture outward.
const BLOCK_SCOPED: &[&str] =
    &["block", "lambda_expression", "anonymous_method_expression", "switch_body"];

/// Expressions whose given named fields are member references.
const EXCLUDE_FIELDS: &[(&str, &str)] = &[("member_access_expression", "name")];

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

    /// Walk children except the given named field (the member slot).
    fn walk_children_excluding_fields(&mut self, n: Node, scope: usize, skip: &[&str]) {
        let skipped: Vec<_> = skip
            .iter()
            .filter_map(|f| n.child_by_field_name(f).map(|c| c.id()))
            .collect();
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if !skipped.contains(&child.id()) {
                self.walk(child, scope);
            }
        }
    }

    fn bind_var(&mut self, name_node: Node, scope: usize, w: Write, intro: IntroKind) {
        let name = self.text(name_node).to_string();
        self.model.bind(scope, &name, w, intro);
    }

    /// Bind a `variable_declarator`'s `@name`, optionally linking its
    /// initializer as the inlinable RHS.
    fn bind_declarator(&mut self, n: Node, scope: usize) {
        if let Some(name) = n.child_by_field_name("name")
            && name.kind() == "identifier"
        {
            // this grammar gives the initializer no field: it is the
            // first named child after the `=` token
            let mut c = n.walk();
            let mut after_eq = false;
            let rhs = n
                .children(&mut c)
                .find(|ch| {
                    if !ch.is_named() && self.text(*ch) == "=" {
                        after_eq = true;
                        false
                    } else {
                        after_eq && ch.is_named()
                    }
                })
                .map(|v| v.id());
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
            "method_declaration" | "constructor_declaration" | "accessor_declaration" => {
                let s = self.model.open_scope(ScopeKind::Function, scope);
                self.walk_children(n, s);
            }
            "local_declaration_statement" | "variable_declaration" => {
                // handled at the declarator level; the statement wrapper
                // only carries type + semicolon
                if kind == "variable_declaration" {
                    let mut cursor = n.walk();
                    let children: Vec<_> = n.children(&mut cursor).collect();
                    for child in children {
                        match child.kind() {
                            "variable_declarator" => self.bind_declarator(child, scope),
                            _ => self.walk(child, scope),
                        }
                    }
                } else {
                    self.walk_children(n, scope);
                }
            }
            "assignment_expression" => {
                let left = n.child_by_field_name("left");
                let right = n.child_by_field_name("right");
                let plain = n.child_by_field_name("operator").and_then(|o| o.utf8_text(self.src).ok()) == Some("=");
                if let Some(left) = left {
                    if left.kind() == "identifier" {
                        let name = self.text(left).to_string();
                        // C# has no undeclared locals: assigning a name
                        // that no visible binding introduced targets a
                        // field/outer symbol -- operands are reads only
                        if self
                            .model
                            .lookup(scope, left.start_byte(), &name)
                            .is_none()
                        {
                            self.walk_children(left, scope);
                        } else {
                            let (w, intro) = if plain {
                                (
                                    Write::assign(
                                        left.start_byte(),
                                        left.id(),
                                        right.map(|r| r.id()),
                                    ),
                                    IntroKind::Assign,
                                )
                            } else {
                                (
                                    Write::rewrite(left.start_byte(), left.id()),
                                    IntroKind::Binding,
                                )
                            };
                            self.model.bind(scope, &name, w, intro);
                            if !plain {
                                self.model.record_read(scope, &name, left.end_byte());
                            }
                        }
                    } else {
                        self.walk_children(left, scope);
                    }
                }
                if let Some(right) = right {
                    self.walk(right, scope);
                }
            }
            // out var / declaration expressions bind and pass-through
            "declaration_expression" => {
                if let Some(d) = n.child_by_field_name("name") {
                    let w = Write::rewrite(d.start_byte(), d.id());
                    self.bind_var(d, scope, w, IntroKind::Binding);
                }
            }
            "foreach_statement" => {
                // the control variable is loop protocol -- never tracked;
                // the iterated collection still produces its reads
                let s = self.model.open_scope(ScopeKind::Block, scope);
                self.walk_children_excluding_fields(n, s, &["left"]);
            }
            "catch_clause" => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                // bind the catch declaration's name, exclude the whole
                // declaration so its identifier never registers a read
                let decl = child_of_kind(n, "catch_declaration");
                if let Some(decl) = decl
                    && let Some(name) = decl.child_by_field_name("name")
                {
                    let w = Write::rewrite(name.start_byte(), name.id());
                    self.bind_var(name, s, w, IntroKind::Binding);
                }
                let skipped = decl.map(|d| d.id());
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if Some(child.id()) != skipped {
                        self.walk(child, s);
                    }
                }
            }
            // ++/-- read and rewrite; other prefix/postfix ops are reads
            k if k.ends_with("unary_expression") => {
                let incdec = matches!(self.op_text(n), Some("++" | "--"));
                let operand = n.named_child(0);
                if let Some(operand) = operand.filter(|o| o.kind() == "identifier" && incdec) {
                    let byte = operand.start_byte();
                    let w = Write::rewrite(byte, operand.id());
                    let name = self.text(operand).to_string();
                    self.model.bind(scope, &name, w, IntroKind::Binding);
                    self.model.record_read(scope, &name, byte + 1);
                } else {
                    self.walk_children(n, scope);
                }
            }
            "identifier" => {
                let name = self.text(n).to_string();
                self.model.record_read(scope, &name, n.start_byte());
            }
            _ => self.walk_children(n, scope),
        }
    }

    fn op_text<'s>(&'s self, n: Node<'s>) -> Option<&'s str> {
        let mut c = n.walk();
        n.children(&mut c)
            .find(|ch| !ch.is_named())
            .and_then(|ch| ch.utf8_text(self.src).ok())
    }
}

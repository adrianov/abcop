//! Tree walk that builds the scope tree: scopes, writes, and reads.

use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Tree};

use super::scope::{RustFile, Scope, ScopeKind};
use super::skip_subtree;

pub(super) struct Builder<'m> {
    pub(super) src: &'m [u8],
    pub(super) scopes: &'m mut Vec<Scope>,
    pub(super) macro_depth: usize,
}

pub fn build(src: &[u8], tree: Tree) -> RustFile<'_> {
    let mut scopes = vec![Scope {
        parent: None,
        kind: ScopeKind::Root,
        entries: HashMap::new(),
    }];
    {
        let mut b = Builder {
            src,
            scopes: &mut scopes,
            macro_depth: 0,
        };
        b.walk(tree.root_node(), 0);
    }
    RustFile { src, tree, scopes }
}

impl<'m> Builder<'m> {
    const SCOPED: [&'static str; 3] = ["function_item", "closure_expression", "block"];
    const BINDERS: [&'static str; 7] = [
        "let_declaration",
        "assignment_expression",
        "compound_assignment_expr",
        "for_expression",
        "if_let_expression",
        "while_let_expression",
        "match_arm",
    ];

    pub(super) fn text<'t>(&'t self, n: Node<'t>) -> &'t str {
        n.utf8_text(self.src).unwrap_or("")
    }

    pub(super) fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();

        if skip_subtree(kind) || kind == "comment" {
            return;
        }

        if Self::SCOPED.contains(&kind) {
            return self.walk_scoped(n, scope, kind);
        }
        if Self::BINDERS.contains(&kind) {
            return self.walk_binder(n, scope, kind);
        }
        self.walk_other(n, scope, kind);
    }

    fn walk_scoped(&mut self, n: Node, scope: usize, kind: &str) {
        let block_scoped = kind != "function_item";
        let s = self.open_scope(
            if block_scoped {
                ScopeKind::Block
            } else {
                ScopeKind::Function
            },
            if block_scoped { Some(scope) } else { None },
        );
        if let Some(p) = n.child_by_field_name("parameters") {
            self.declare_params(p, s);
        }
        if block_scoped && kind == "closure_expression" {
            if let Some(body) = n.child_by_field_name("body") {
                self.walk(body, s);
            }
            return;
        }
        if kind == "block" {
            self.walk_children(n, s);
            return;
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.walk_children(body, s);
        }
    }

    fn walk_binder(&mut self, n: Node, scope: usize, kind: &str) {
        match kind {
            "let_declaration" => self.handle_let(n, scope),
            "assignment_expression" | "compound_assignment_expr" => self.handle_assign(n, scope),
            "match_arm" => self.handle_match_arm(n, scope),
            _ => self.handle_loop_or_let_binding(n, scope),
        }
    }

    fn walk_other(&mut self, n: Node, scope: usize, kind: &str) {
        if matches!(
            kind,
            "string_literal" | "raw_string_literal" | "c_string_literal"
        ) {
            // format strings implicitly capture named arguments as reads
            self.record_format_captures(n, scope);
            return;
        }
        if kind == "token_tree" {
            self.macro_depth += 1;
            self.walk_children(n, scope);
            self.macro_depth -= 1;
            return;
        }
        if kind == "identifier" {
            self.walk_ident_node(n, scope);
            return;
        }
        if kind == "scoped_identifier" {
            return;
        }
        self.walk_children(n, scope);
    }

    fn walk_ident_node(&mut self, n: Node, scope: usize) {
        let name = self.text(n).to_string();
        if name.starts_with('_') {
            return;
        }
        if self.lookup(scope, n.start_byte(), &name).is_some() {
            self.record_read(scope, &name, n.start_byte());
        }
    }

    pub(super) fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    pub(super) fn walk_skip_ids(&mut self, n: Node, scope: usize, skip: &HashSet<usize>) {
        if skip.contains(&n.id()) {
            return;
        }
        if skip_subtree(n.kind()) || n.kind() == "comment" {
            return;
        }
        if n.child_count() == 0 {
            self.walk(n, scope);
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk_skip_ids(child, scope, skip);
        }
    }
}

//! Collector walk for C#: the language-specific arms. Generic
//! dispatch (skip subtrees, member-slot exclusions, Block scopes,
//! Function boundaries, plain identifier reads) lives in
//! [`crate::scope_model::walk`].
//!
//! C# notes: bare identifiers assigned with no visible binding target
//! fields/outer symbols -- C# has no undeclared locals -- so such
//! assignments contribute operand reads only; foreach heads and catch
//! declarations are protocol.

use tree_sitter::Node;

use crate::scope_model::walk::{dispatch, Backend, Spec};
use crate::scope_model::{child_of_kind, IntroKind, Model, ScopeKind, Write};

static SPEC: Spec = Spec {
    skip_kinds: &[
        "parameter_list",
        "field_declaration",
        "using_directive",
        "namespace_declaration",
        "file_scoped_namespace_declaration",
        "qualified_name",
    ],
    block_scoped: &[
        "block",
        "lambda_expression",
        "anonymous_method_expression",
        "switch_body",
    ],
    function_kinds: &["method_declaration", "constructor_declaration", "accessor_declaration"],
    exclude_fields: &[("member_access_expression", "name")],
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
            // `var x = ...;` / `int a, b = 0;`
            "variable_declaration" => {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    if child.kind() == "variable_declarator" {
                        self.bind_declarator_with_rhs_field(child, scope);
                    } else {
                        dispatch(self, child, scope);
                    }
                }
            }
            "assignment_expression" => self.walk_assignment(n, scope),
            // `out var x` / declaration expressions: bind and pass --
            // they read through the `out`, never inline candidates
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
                self.walk_children_excluding_left(n, s);
            }
            "catch_clause" => {
                let s = self.model.open_scope(ScopeKind::Block, scope);
                // bind the declaration's name but exclude the whole
                // declaration node: its identifier must never leak into
                // the read path
                let decl = child_of_kind(n, "catch_declaration");
                if let Some(decl) = decl
                    && let Some(name) = decl.child_by_field_name("name")
                {
                    let w = Write::rewrite(name.start_byte(), name.id());
                    self.bind_var(name, s, w, IntroKind::Binding);
                }
                let skipped = decl.map(|d| d.id());
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                for child in children {
                    if Some(child.id()) != skipped {
                        dispatch(self, child, s);
                    }
                }
            }
            // everything else: generic dispatch over the children --
            // including local_declaration_statement wrappers
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
    fn walk(&mut self, n: Node, scope: usize) {
        dispatch(self, n, scope);
    }

    fn walk_children_excluding_left(&mut self, n: Node, scope: usize) {
        let skipped = n.child_by_field_name("left").map(|c| c.id());
        let mut cursor = n.walk();
        let children: Vec<_> = n.children(&mut cursor).collect();
        for child in children {
            if Some(child.id()) != skipped {
                dispatch(self, child, scope);
            }
        }
    }

    /// Plain `=` rebinds a visible local (one candidate write);
    /// compound operators rewrite-and-read. Assignments to names no
    /// visible binding introduced target fields or outer symbols --
    /// C# has no undeclared locals -- so their operands are reads only,
    /// and the assigned field itself is never registered as a local.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain =
            n.child_by_field_name("operator").and_then(|o| o.utf8_text(self.src).ok()) == Some("=");
        let Some(left) = left else { return };

        if left.kind() != "identifier" {
            // member/indexer targets: operands are reads
            let mut cursor = left.walk();
            let children: Vec<_> = left.children(&mut cursor).collect();
            for child in children {
                dispatch(self, child, scope);
            }
        } else {
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
        }

        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }
}

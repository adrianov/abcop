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
    read_kinds: &["identifier"],
    exclude_fields: &[("member_access_expression", "name")],
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
            // `var x = ...;` / `int a, b = 0;`
            "variable_declaration" => self.bind_variable_declarations(n, scope),
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
                self.walk_children_excluding_field(n, s, "left");
            }
            "catch_clause" => self.bind_catch_clause(n, scope),
            // everything else: generic dispatch over the children --
            // including local_declaration_statement wrappers
            _ => self.walk_children(n, scope),
        }
    }
}

impl Collector<'_> {
    /// Bind each `variable_declarator` of a C# declaration statement;
    /// wrapper tokens and modifiers pass through the generic walk.
    fn bind_variable_declarations(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                self.bind_declarator_with_rhs_field(child, scope);
            } else {
                dispatch(self, child, scope);
            }
        }
    }

    /// A catch handler opens a block scope; its exception variable is
    /// bound as a rewrite (never an inline candidate) while the rest of
    /// the declaration node must not leak into the read path.
    fn bind_catch_clause(&mut self, n: Node, scope: usize) {
        let s = self.model.open_scope(ScopeKind::Block, scope);
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
                dispatch(self, child, s);
            }
        }
    }

    /// Plain `=` rebinds a visible local (one candidate write);
    /// compound operators rewrite-and-read. Assignments to names no
    /// visible binding introduced target fields or outer symbols --
    /// C# has no undeclared locals -- so their operands are reads only,
    /// and the assigned field itself is never registered as a local.
    /// The `[`left`/`right`] field family is the JS one.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain =
            n.child_by_field_name("operator").and_then(|o| o.utf8_text(self.src).ok()) == Some("=");
        if let Some(left) = left {
            if left.kind() == "identifier" {
                self.rebind_local(left, scope, plain, right.map(|r| r.id()));
            } else {
                // member/indexer targets: operands are reads
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }
}

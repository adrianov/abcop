//! Collector walk for Solidity: which nodes open scopes, which bind or
//! read variables. Evaluation lives in [`crate::scope_model`].
//!
//! Solidity notes: assignments to names no visible binding introduced
//! target state variables -- Solidity has no undeclared locals either,
//! so such targets contribute operand reads only; member `@property`
//! slots are never variable reads.

use tree_sitter::Node;

use crate::scope_model::walk::{dispatch, Backend};
use crate::scope_model::walk::Spec;
use crate::scope_model::{child_of_kind, IntroKind, Model, Scope, Write};

/// Static description of the Solidity walk.
static SPEC: Spec = Spec {
    skip_kinds: &[
        "parameter",
        "state_variable_declaration",
        "struct_declaration",
        "enum_declaration",
        "event_definition",
        "error_definition",
        "using_directive",
        "pragma_directive",
        "import_directive",
    ],
    block_scoped: &["block_statement", "unchecked_block"],
    function_kinds: &[
        "function_definition",
        "constructor_definition",
        "modifier_definition",
    ],
    exclude_fields: &[("member_expression", "property")],
};

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
            // `uint256 x = expr;` and tuple heads `(bool ok, ) = ...`
            "variable_declaration_statement" => {
                let mut cursor = n.walk();
                let children: Vec<_> = n.children(&mut cursor).collect();
                let mut decls: Vec<_> = Vec::new();
                let mut value: Option<Node> = None;
                for child in children {
                    let k = child.kind();
                    match k {
                        "variable_declaration" | "variable_declaration_tuple" => {
                            collect_decls(child, &mut decls);
                        }
                        ";" | "=" => {}
                        _ if child.is_named() => value = Some(child),
                        _ => {}
                    }
                }
                let single_pair = decls.len() == 1 && value.is_some();
                for d in &decls {
                    if let Some(name) = d.child_by_field_name("name") {
                        let rhs = if single_pair { value.map(|v| v.id()) } else { None };
                        let w = Write::assign(name.start_byte(), name.id(), rhs);
                        self.bind_var(name, scope, w, IntroKind::Assign);
                    }
                }
                if let Some(v) = value {
                    dispatch(self, v, scope);
                }
            }
            "assignment_expression" => self.walk_assignment(n, scope),
            "augmented_assignment_expression" => {
                if let Some(target) = plain_identifier_target(n) {
                    let w = Write::rewrite(target.start_byte(), target.id());
                    self.bind_var(target, scope, w, IntroKind::Binding);
                    self.read_at(scope, target, target.end_byte());
                } else {
                    let mut cursor = n.walk();
                    let children: Vec<_> = n.children(&mut cursor).collect();
                    for child in children {
                        dispatch(self, child, scope);
                    }
                }
            }
            "update_expression" => {
                // i++ / --i read and rewrite
                if let Some(operand) = child_of_kind(n, "identifier") {
                    let w = Write::rewrite(operand.start_byte(), operand.id());
                    self.bind_var(operand, scope, w, IntroKind::Binding);
                    self.read_at(scope, operand, operand.end_byte());
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
    fn walk(&mut self, n: Node, scope: usize) {
        dispatch(self, n, scope);
    }

    /// Plain `=` rebinds a visible local (one candidate write);
    /// compound operators rewrite-and-read. Assignments to names no
    /// visible binding introduced target state variables -- Solidity
    /// has no undeclared locals -- so their operands are reads only.
    fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let mut cursor = n.walk();
        let op = n
            .children(&mut cursor)
            .find(|ch| !ch.is_named())
            .map(|ch| ch.utf8_text(self.src).unwrap_or("").to_string())
            .unwrap_or_default();
        let plain = op == "=";
        let Some(left) = left else { return };

        let mut targets: Vec<Node> = Vec::new();
        plain_identifier_targets(left, &mut targets);

        if targets.is_empty() {
            // state mapping / member write: operands are reads only
            let mut lc = left.walk();
            let operands: Vec<_> = left.children(&mut lc).collect();
            for operand in operands {
                dispatch(self, operand, scope);
            }
        } else {
            let single_pair = targets.len() == 1;
            for t in &targets {
                let rhs = if plain && single_pair {
                    right.map(|r| r.id())
                } else {
                    None
                };
                let w = Write::assign(t.start_byte(), t.id(), rhs);
                self.bind_var(*t, scope, w, IntroKind::Assign);
                if !plain {
                    self.read_at(scope, *t, t.end_byte());
                }
            }
        }

        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }

    fn read_at(&mut self, scope: usize, name_node: Node, byte: usize) {
        let name = self.text_of(name_node).to_string();
        self.model.record_read(scope, &name, byte);
    }
}

/// Identifier targets bound by an assignment head. Tuple heads expand
/// per declared name; member/array/call targets reference their
/// operands instead and bind nothing.
fn plain_identifier_targets<'t>(n: Node<'t>, mut out: &mut Vec<Node<'t>>) {
    match n.kind() {
        "identifier" => out.push(n),
        "member_expression" | "array_access" | "call_expression" => {}
        _ => {
            let mut cursor = n.walk();
            let children: Vec<_> = n.children(&mut cursor).collect();
            for child in children {
                plain_identifier_targets(child, &mut out);
            }
        }
    }
}

fn plain_identifier_target(n: Node) -> Option<Node> {
    let mut out = Vec::new();
    plain_identifier_targets(n, &mut out);
    if out.len() == 1 {
        out.pop()
    } else {
        None
    }
}

fn collect_decls<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    match n.kind() {
        "variable_declaration" => out.push(n),
        "variable_declaration_tuple" => {
            let mut cursor = n.walk();
            let children: Vec<_> = n.children(&mut cursor).collect();
            for child in children {
                collect_decls(child, out);
            }
        }
        _ => {}
    }
}

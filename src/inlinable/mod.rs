//! Shared RHS inlinability: literals/compositions, call/index chains, and
//! bare identifier reads (with optional reassignment guard for UsedOnce).

mod ruby;

pub use ruby::ruby_inlinable_rhs;

use tree_sitter::Node;

use crate::scope_model::{Scope, Semantics};

pub const RUBY_UNITS: &[&str] = &["call", "element_reference"];
pub const RUBY_IDENT: &str = "identifier";

pub const JS_UNITS: &[&str] = &[
    "call_expression",
    "member_expression",
    "subscript_expression",
    "new_expression",
];
pub const JS_IDENT: &str = "identifier";

pub const SWIFT_UNITS: &[&str] = &[
    "call_expression",
    "navigation_expression",
    "subscript_expression",
];
pub const SWIFT_IDENT: &str = "simple_identifier";

pub const C_UNITS: &[&str] = &[
    "call_expression",
    "field_expression",
    "subscript_expression",
    "pointer_expression",
];
pub const C_IDENT: &str = "identifier";

pub const JAVA_UNITS: &[&str] = &["method_invocation", "field_access", "array_access"];
pub const JAVA_IDENT: &str = "identifier";

pub const CSHARP_UNITS: &[&str] = &[
    "invocation_expression",
    "member_access_expression",
    "element_access_expression",
    "object_creation_expression",
];
pub const CSHARP_IDENT: &str = "identifier";

pub const PHP_UNITS: &[&str] = &[
    "function_call_expression",
    "member_call_expression",
    "scoped_call_expression",
    "nullsafe_member_call_expression",
];
pub const PHP_IDENT: &str = "variable_name";

pub const DART_UNITS: &[&str] = &[
    "call_expression",
    "function_expression",
    "instance_creation_expression",
    "selector",
    "index_expression",
    "cascade_section",
];
pub const DART_IDENT: &str = "identifier";

pub const ZIG_UNITS: &[&str] = &["call_expression", "field_access", "array_access", "slice"];
pub const ZIG_IDENT: &str = "identifier";

pub const SOL_UNITS: &[&str] = &["call_expression", "member_expression", "index_access"];
pub const SOL_IDENT: &str = "identifier";

pub const PY_UNITS: &[&str] = &["call", "attribute", "subscript"];
pub const PY_IDENT: &str = "identifier";

pub const GO_UNITS: &[&str] = &[
    "call_expression",
    "selector_expression",
    "index_expression",
    "type_assertion_expression",
];
pub const GO_IDENT: &str = "identifier";

pub const RUST_UNITS: &[&str] = &[
    "call_expression",
    "method_call_expression",
    "field_expression",
    "index_expression",
    "await_expression",
];
pub const RUST_IDENT: &str = "identifier";

/// Parents whose named children are sequential statements.
const STMT_LIST_PARENTS: &[&str] = &[
    "body_statement",
    "block_body",
    "statement_block",
    "statement_list",
    "statements",
    "block",
    "compound_statement",
    "function_body",
    "program",
    "source_file",
    "declaration_list",
];

/// Bodies that may run more than once between assignment and read.
const REPEAT_ANCESTORS: &[&str] = &[
    "do_block",
    "lambda",
    "closure_expression",
    "for_statement",
    "for_in_statement",
    "for_of_statement",
    "while_statement",
    "do_statement",
    "repeat_while_statement",
    "for_expression",
    "while_expression",
    "loop_expression",
    "enhanced_for_statement",
    "foreach_statement",
];

/// Branch bodies that may skip evaluating the read (Ruby `then`/`when`, etc.).
const BRANCH_BODY_KINDS: &[&str] = &[
    "then", "else", "elsif", "when", "in_clause", "rescue", "ensure",
];

/// Effectful RHS may move to the read only when it is the next statement and
/// the read always runs there (not under a loop, conditional branch, or
/// modifier body — those change when/whether the call executes).
pub fn immediate_substitutable(write_site: Node, read_byte: usize) -> bool {
    let Some(read_site) = root_of(write_site).descendant_for_byte_range(read_byte, read_byte) else {
        return false;
    };
    let (write_stmt, write_idx) = match list_member(write_site) {
        Some(pair) => pair,
        None => return false,
    };
    let (read_stmt, read_idx) = match list_member(read_site) {
        Some(pair) => pair,
        None => return false,
    };
    let (Some(write_parent), Some(read_parent)) = (write_stmt.parent(), read_stmt.parent()) else {
        return false;
    };
    if write_parent.id() != read_parent.id() || read_idx != write_idx + 1 {
        return false;
    }
    !read_may_skip(read_site, read_stmt)
}

fn root_of(mut node: Node) -> Node {
    while let Some(parent) = node.parent() {
        node = parent;
    }
    node
}

fn list_member(node: Node) -> Option<(Node, usize)> {
    let mut cur = node;
    loop {
        let parent = cur.parent()?;
        if STMT_LIST_PARENTS.contains(&parent.kind()) {
            let idx = sibling_index(parent, cur)?;
            return Some((cur, idx));
        }
        cur = parent;
    }
}

fn sibling_index(parent: Node, child: Node) -> Option<usize> {
    let mut cursor = parent.walk();
    let mut idx = 0;
    for c in parent.children(&mut cursor) {
        if !c.is_named() {
            continue;
        }
        if c.id() == child.id() {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

/// True when the read sits under control flow that may skip it relative to
/// `read_stmt` (loops, `then`/`else`, modifier bodies, ternary arms).
/// Reads only in a condition stay reachable — the condition always runs.
fn read_may_skip(read_site: Node, read_stmt: Node) -> bool {
    let mut cur = Some(read_site);
    while let Some(n) = cur {
        if n.id() == read_stmt.id() {
            return false;
        }
        if REPEAT_ANCESTORS.contains(&n.kind()) || BRANCH_BODY_KINDS.contains(&n.kind()) {
            return true;
        }
        if let Some(parent) = n.parent() {
            if modifier_body_child(parent, n) || conditional_branch_child(parent, n) {
                return true;
            }
        }
        cur = n.parent();
    }
    false
}

fn modifier_body_child(parent: Node, child: Node) -> bool {
    matches!(
        parent.kind(),
        "if_modifier" | "unless_modifier" | "while_modifier" | "until_modifier" | "rescue_modifier"
    ) && under_field(parent, "body", child)
}

fn conditional_branch_child(parent: Node, child: Node) -> bool {
    matches!(parent.kind(), "if" | "unless" | "conditional")
        && (under_field(parent, "consequence", child) || under_field(parent, "alternative", child))
}

fn under_field(parent: Node, field: &str, child: Node) -> bool {
    let Some(field_node) = parent.child_by_field_name(field) else {
        return false;
    };
    if child.id() == field_node.id() {
        return true;
    }
    let mut cur = child.parent();
    while let Some(n) = cur {
        if n.id() == field_node.id() {
            return true;
        }
        if n.id() == parent.id() {
            return false;
        }
        cur = n.parent();
    }
    false
}

/// A binding read on the RHS is safe when the source local is not written
/// between `write_byte` and `read_byte`. Unresolved names are vcalls.
pub fn alias_stable(
    scopes: &[Scope],
    scope: usize,
    pos: usize,
    name: &str,
    write_byte: usize,
    read_byte: usize,
) -> bool {
    match lookup_binding(scopes, scope, pos, name) {
        None => true,
        Some(bind_scope) => scopes[bind_scope]
            .entries
            .get(name)
            .is_none_or(|entry| {
                !entry
                    .writes
                    .iter()
                    .any(|w| w.byte > write_byte && w.byte < read_byte)
            }),
    }
}

/// Peel grammar wrappers (`expression`, parens) so unit-kind checks see
/// the call/index node Solidity and similar grammars nest under them.
fn peel_rhs(n: Node) -> Node {
    let mut cur = n;
    while cur.named_child_count() == 1
        && matches!(cur.kind(), "expression" | "parenthesized_expression")
    {
        cur = cur.named_child(0).expect("named_child_count == 1");
    }
    cur
}

/// RHS may replace a read site or stand alone when the binding is dropped.
pub fn rhs_inlinable(
    src: &[u8],
    n: Node,
    sem: &Semantics,
    scopes: &[Scope],
    scope: usize,
    write_byte: usize,
    read_byte: Option<usize>,
    write_site: Option<Node>,
) -> bool {
    let n = peel_rhs(n);
    if sem.unit_kinds.contains(&n.kind()) {
        return match read_byte {
            None => true,
            Some(rb) => write_site.is_some_and(|site| immediate_substitutable(site, rb)),
        };
    }
    if n.kind() == sem.ident_kind {
        return match read_byte {
            Some(end) => alias_stable(
                scopes,
                scope,
                write_byte,
                n.utf8_text(src).unwrap_or(""),
                write_byte,
                end,
            ),
            None => true,
        };
    }
    (sem.pure)(n)
}

/// True when the initializer is a call/index chain to keep as a statement.
pub fn keep_init_rhs(n: Node, sem: &Semantics) -> bool {
    sem.unit_kinds.contains(&peel_rhs(n).kind())
}

pub fn keep_init_kind(n: Node, unit_kinds: &[&str]) -> bool {
    unit_kinds.contains(&peel_rhs(n).kind())
}

fn lookup_binding(scopes: &[Scope], scope: usize, pos: usize, name: &str) -> Option<usize> {
    let data = &scopes[scope];
    if let Some(e) = data.entries.get(name) {
        return if e.intro_byte <= pos {
            Some(scope)
        } else {
            None
        };
    }
    match data.kind {
        crate::scope_model::ScopeKind::Block => lookup_binding(scopes, data.parent?, pos, name),
        _ => None,
    }
}

//! Bind filters for C-family locals: reject misparsed keywords/macros and
//! RAII lifetime guards that NeverUsed would wrongly flag.

use tree_sitter::Node;

/// Keywords / nullability macros that tree-sitter may emit as bogus
/// declarators (e.g. `#ifdef` splitting `else if`, ObjC headers on the
/// C++ grammar).
pub(super) fn should_bind(init_decl: Node, name: &str, rhs: Option<Node>, src: &[u8]) -> bool {
    bindable_local_name(name) && !is_lifetime_guard(init_decl, rhs, src)
}

fn bindable_local_name(name: &str) -> bool {
    !matches!(
        name,
        "if" | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "return"
            | "goto"
            | "new"
            | "delete"
            | "sizeof"
            | "typeof"
            | "typedef"
            | "struct"
            | "class"
            | "union"
            | "enum"
            | "template"
            | "typename"
            | "NS_ASSUME_NONNULL_BEGIN"
            | "NS_ASSUME_NONNULL_END"
    )
}

/// True when the binding exists only to run a destructor / extend a
/// temporary's lifetime (`lock_guard`, `unique_lock()`, `shared_from_this`,
/// `QSignalBlocker{...}`).
fn is_lifetime_guard(init_decl: Node, rhs: Option<Node>, src: &[u8]) -> bool {
    decl_type_is_guard(init_decl, src) || rhs.is_some_and(|r| rhs_is_guard(r, src))
}

fn decl_type_is_guard(init_decl: Node, src: &[u8]) -> bool {
    init_decl
        .parent()
        .filter(|d| matches!(d.kind(), "declaration" | "field_declaration"))
        .and_then(|d| d.child_by_field_name("type"))
        .is_some_and(|ty| type_mentions_guard(ty, src))
}

fn type_mentions_guard(n: Node, src: &[u8]) -> bool {
    match n.kind() {
        "type_identifier" | "identifier" | "field_identifier" => {
            is_guard_type_name(n.utf8_text(src).unwrap_or(""))
        }
        "qualified_identifier" | "template_type" => n
            .child_by_field_name("name")
            .is_some_and(|name| type_mentions_guard(name, src)),
        _ => false,
    }
}

fn rhs_is_guard(n: Node, src: &[u8]) -> bool {
    match n.kind() {
        "call_expression" => n
            .child_by_field_name("function")
            .is_some_and(|f| callee_is_guard(f, src)),
        "compound_literal_expression" => n
            .child_by_field_name("type")
            .is_some_and(|t| type_mentions_guard(t, src)),
        "parenthesized_expression" => n.named_child(0).is_some_and(|c| rhs_is_guard(c, src)),
        _ => false,
    }
}

fn callee_is_guard(n: Node, src: &[u8]) -> bool {
    match n.kind() {
        "identifier" | "field_identifier" | "type_identifier" => {
            is_guard_call_name(n.utf8_text(src).unwrap_or(""))
        }
        "field_expression" | "qualified_identifier" | "template_function" => n
            .child_by_field_name(if n.kind() == "field_expression" {
                "field"
            } else {
                "name"
            })
            .is_some_and(|c| callee_is_guard(c, src)),
        _ => false,
    }
}

fn is_guard_type_name(name: &str) -> bool {
    matches!(
        name,
        "lock_guard"
            | "unique_lock"
            | "scoped_lock"
            | "shared_lock"
            | "unique_ptr"
            | "shared_ptr"
            | "QSignalBlocker"
            | "ScopeGuard"
    )
}

fn is_guard_call_name(name: &str) -> bool {
    is_guard_type_name(name) || name == "shared_from_this"
}

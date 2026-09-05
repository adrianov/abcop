//! Bind filters for C-family locals: reject misparsed keywords/macros and
//! RAII lifetime guards that NeverUsed would wrongly flag.

use tree_sitter::Node;

/// Keywords / nullability macros that tree-sitter may emit as bogus
/// declarators (e.g. `#ifdef` splitting `else if`, ObjC headers on the
/// C++ grammar).
pub(super) fn should_bind(init_decl: Node, name: &str, rhs: Option<Node>, src: &[u8]) -> bool {
    bindable_local_name(name)
        && !parent_skips_declaration_bind(init_decl, src)
        && !is_lifetime_guard(init_decl, rhs, src)
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

/// True when a `declaration` is a statement macro or a misparsed expression
/// (`emit`, `paths[I] = …`, adjacent `MACRO MACRO;`). Walk children; do not bind.
pub(super) fn skip_declaration_bind(decl: Node, src: &[u8]) -> bool {
    matches!(decl.kind(), "declaration" | "field_declaration")
        && (decl
            .child_by_field_name("type")
            .is_some_and(|ty| type_is_statement_macro(ty, src))
            || is_expression_like_declaration(decl, src))
}

/// Same check from an `init_declarator` / bare name under that declaration.
fn parent_skips_declaration_bind(init_decl: Node, src: &[u8]) -> bool {
    init_decl
        .parent()
        .is_some_and(|d| skip_declaration_bind(d, src))
}

/// Error-recovery shapes that look like declarations but are expressions.
fn is_expression_like_declaration(decl: Node, src: &[u8]) -> bool {
    let Some(ty) = decl.child_by_field_name("type") else {
        return false;
    };
    if ty.kind() != "type_identifier" {
        return false;
    }
    // `paths[PATH_LOCALE] = rhs` → type `paths` + structured_binding `[PATH_LOCALE]`.
    // Real structured bindings use `auto` / placeholder types, not a bare id.
    if declaration_has_structured_binding(decl) {
        return true;
    }
    // `"/str" LOCALE_DIR PATH_SEPARATOR_STR` → `LOCALE_DIR PATH_SEPARATOR_STR;`
    let ty_name = ty.utf8_text(src).unwrap_or("");
    bare_declarator_name(decl, src).is_some_and(|n| {
        is_macroish(ty_name) && is_macroish(n) && !declaration_has_initializer(decl)
    })
}

fn declaration_has_structured_binding(decl: Node) -> bool {
    let mut c = decl.walk();
    decl.children(&mut c).any(|ch| {
        ch.kind() == "structured_binding_declarator"
            || (ch.kind() == "init_declarator"
                && ch
                    .child_by_field_name("declarator")
                    .is_some_and(|d| d.kind() == "structured_binding_declarator"))
    })
}

fn declaration_has_initializer(decl: Node) -> bool {
    decl.child_by_field_name("value").is_some()
        || {
            let mut c = decl.walk();
            decl.children(&mut c).any(|ch| {
                ch.kind() == "init_declarator" && ch.child_by_field_name("value").is_some()
            })
        }
}

fn bare_declarator_name<'a>(decl: Node<'a>, src: &'a [u8]) -> Option<&'a str> {
    let mut c = decl.walk();
    decl.children(&mut c)
        .find(|ch| ch.kind() == "identifier")
        .and_then(|id| id.utf8_text(src).ok())
}

fn is_macroish(name: &str) -> bool {
    name.len() > 1
        && name.contains('_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn type_is_statement_macro(n: Node, src: &[u8]) -> bool {
    match n.kind() {
        "type_identifier" | "identifier" => {
            is_statement_macro_name(n.utf8_text(src).unwrap_or(""))
        }
        _ => false,
    }
}

fn is_statement_macro_name(name: &str) -> bool {
    matches!(
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
            | "emit"
            | "Q_EMIT"
            | "DEBUG_BLOCK"
            | "foreach"
            | "forever"
            | "Q_FOREACH"
            | "NS_ASSUME_NONNULL_BEGIN"
            | "NS_ASSUME_NONNULL_END"
    )
}

/// True when the binding exists only to run a destructor / extend a
/// temporary's lifetime (`lock_guard`, `unique_lock()`, `shared_from_this`,
/// `QSignalBlocker{...}`, `QMutexLocker`, `HashPauser`).
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
            | "QMutexLocker"
            | "QReadLocker"
            | "QWriteLocker"
            | "HashPauser"
            | "ScopeGuard"
            // DC++ / eiskalt CriticalSection holders; create-only `File f(...)`
            | "Lock"
            | "FastLock"
            | "File"
    )
}

fn is_guard_call_name(name: &str) -> bool {
    is_guard_type_name(name) || matches!(name, "shared_from_this" | "lock")
}

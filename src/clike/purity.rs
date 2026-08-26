//! Conservative RHS purity predicates powering UsedOnce candidate
//! filtering across the clike backends.
//!
//! A binding is an inline candidate only when its initializer is provably
//! side-effect-free: literals, operator compositions over literals, and
//! template strings without substitutions. References to other locals,
//! calls, member reads, and closure/capture expressions all disqualify,
//! since inlining them would change evaluation order or side effects.

use tree_sitter::Node;

/// Walk every named child, asserting `purity` over it. Shared by the
/// language-specific `pure` predicates to avoid re-stating the traversal.
fn children_all_pure(n: Node, purity: fn(Node) -> bool) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| purity(ch))
}

/// True when `n`'s named children are all pure *and* carry no template
/// substitution (Swift has no templates; JS does).
fn children_without_substitution(n: Node, purity: fn(Node) -> bool) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| ch.kind() != "substitution" && purity(ch))
}

/// Conservative RHS purity for JavaScript/TypeScript: literals, operator
/// compositions, and template literals without substitutions. References to
/// other locals, calls, and member reads fail it.
pub(super) fn js_pure(n: Node) -> bool {
    match n.kind() {
        "number" | "string" | "true" | "false" | "null" | "undefined" => {
            children_without_substitution(n, js_pure)
        }
        "template_string" => children_without_substitution(n, js_pure),
        "parenthesized_expression" | "binary_expression" | "unary_expression" | "typeof_expression" => {
            children_all_pure(n, js_pure)
        }
        _ => false,
    }
}

/// Conservative RHS purity for Swift: literal leaves and value-typed
/// compositions thereof. Calls, member reads, other locals, and optional
/// chaining all fail -- inlining must not change evaluation side effects.
pub(super) fn swift_pure(n: Node) -> bool {
    match n.kind() {
        "integer_literal"
        | "float_literal"
        | "boolean_literal"
        | "nil"
        | "line_string_literal"
        | "array_literal"
        | "dictionary_literal"
        | "prefix_expression"
        | "infix_expression" => children_all_pure(n, swift_pure),
        _ => false,
    }
}

/// Conservative RHS purity for the plain C-family grammars (C, C++,
/// Objective-C): literal leaves plus operator compositions thereof.
/// Identifiers (other locals), calls, member reads (`->`/`.`), increments,
/// ternaries, and brace initializers all fail -- inlining must not
/// reorder or duplicate their evaluation nor change object identity.
pub(super) fn c_like_pure(n: Node) -> bool {
    match n.kind() {
        "number_literal"
        | "string_literal"
        | "char_literal"
        | "concatenated_string"
        | "true"
        | "false"
        | "null"
        | "nil"
        | "nullptr" => children_all_pure(n, c_like_pure),
        "parenthesized_expression" | "binary_expression" | "unary_expression" => {
            children_all_pure(n, c_like_pure)
        }
        _ => false,
    }
}

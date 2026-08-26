//! The ABC counter for Dart: kind-classification slices, the running
//! tally and its stateful specials (declarators, assignments, for-in
//! heads, cascade writes). Unit discovery lives in [`super::abc`].

use tree_sitter::Node;

use super::patterns::bare_target;

/// Unit kinds; also the tally boundary set.
const UNIT_KINDS: &[&str] = &[
    "function_declaration",
    "method_declaration",
    "getter_declaration",
    "setter_declaration",
];

/// Invocation-shaped kinds: one B each.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "new_expression",
    "const_object_expression",
    "constructor_invocation",
    "cascade_call_expression",
];

/// Arithmetic/bitwise/shift binary operators: one B each.
const B_BINARY_KINDS: &[&str] = &[
    "additive_expression",
    "multiplicative_expression",
    "bitwise_and_expression",
    "bitwise_or_expression",
    "bitwise_xor_expression",
    "shift_expression",
];

/// Branch/condition shapes: one C each (switch cases and defaults count
/// individually, mirrors switch_section handling in the C# backend).
const BRANCH_KINDS: &[&str] = &[
    "if_statement",
    "while_statement",
    "do_statement",
    "conditional_expression",
    "logical_and_expression",
    "logical_or_expression",
    "equality_expression",
    "relational_expression",
    "if_null_expression",
    "type_test",
    "type_test_expression",
    "type_cast",
    "type_cast_expression",
    "switch_statement_case",
    "switch_statement_default",
    "switch_expression_case",
    "catch_clause",
];

#[derive(Default)]
pub(super) struct Tally<'s> {
    pub(super) src: &'s [u8],
    pub(super) a: u32,
    pub(super) b: u32,
    pub(super) c: u32,
}

impl Tally<'_> {
    /// Operator token of assignment nodes.
    fn op_of(&self, n: Node<'_>) -> Option<&str> {
        n.child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
    }

    /// Anonymous token child (`++`, `--`, cascade `=`).
    fn anon_op(&self, n: Node<'_>) -> Option<&str> {
        n.children(&mut n.walk())
            .filter(|ch| !ch.is_named())
            .find_map(|ch| ch.utf8_text(self.src).ok())
    }

    pub(super) fn walk(&mut self, n: Node) {
        // a named unit never nests inside another Dart unit: pure safety net
        if UNIT_KINDS.contains(&n.kind()) {
            return;
        }
        self.add_special(n);
        self.add_table(n);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child);
        }
    }

    /// Stateful special cases. Kinds handled here are fully consumed;
    /// anything else falls through to the slice-driven categories.
    fn add_special(&mut self, n: Node<'_>) -> bool {
        match n.kind() {
            // one A per declared local
            "initialized_variable_definition" => self.a += 1,
            "pattern_variable_declaration" => self.a += pattern_target_count(n),
            "for_statement" => {
                self.c += 1;
                // for-in head declares its element variable
                self.a += u32::from(n.child_by_field_name("name").is_some());
            }
            "assignment_expression" => self.add_assignment(n),
            // cascade section carrying an instance-field write
            "cascade_section" if self.anon_op(n) == Some("=") => self.a += 1,
            _ => return false,
        }
        true
    }

    /// Plain writes to bare identifiers are the A payload; compound
    /// operators always count, field writes with plain `=` do not
    /// (mirrors the C# backend).
    fn add_assignment(&mut self, n: Node<'_>) {
        let plain = self.op_of(n) == Some("=");
        let bare = n
            .child_by_field_name("left")
            .and_then(bare_target)
            .is_some();
        if bare || !plain {
            self.a += 1;
        }
    }

    /// Slice-driven single-point categories.
    fn add_table(&mut self, n: Node<'_>) -> bool {
        let kind = n.kind();
        if CALL_KINDS.contains(&kind) || B_BINARY_KINDS.contains(&kind) {
            self.b += 1;
        } else if kind == "unary_expression" || kind == "postfix_expression" {
            // `++`/`--` rewrite a variable exactly like Go's inc/dec;
            // every other unary operator is an operation (B)
            match self.anon_op(n) {
                Some("++" | "--") => self.a += 1,
                _ => self.b += 1,
            }
        } else if BRANCH_KINDS.contains(&kind) {
            self.c += 1;
        } else {
            return false;
        }
        true
    }
}

fn pattern_target_count(n: Node) -> u32 {
    fn count(n: Node, out: &mut u32) {
        if n.kind() == "variable_pattern" {
            *out += 1;
        }
        let mut c = n.walk();
        for ch in n.children(&mut c) {
            count(ch, out);
        }
    }
    let mut out = 0;
    count(n, &mut out);
    out
}

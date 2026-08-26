//! Pattern-binding domain for Dart: destructuring declarations and
//! pattern assignments split at their top-level `=` so the pattern side
//! binds names exactly once and never becomes its own reader.
//!
//! Dart notes: in a refutable pattern context only `variable_pattern`
//! declares a fresh binding and constant patterns are equality tests --
//! but on the *assignment* side (`[a, b] = pair;`) every identifier
//! writes. Wildcards `_` are filtered by the model itself.

use tree_sitter::Node;

use crate::scope_model::walk::{Backend, dispatch};

use crate::scope_model::{IntroKind, Write};

use super::scope::{Collector, bare_target};

impl Collector<'_> {
    /// Names declared by a pattern before its `=`: every pattern-side
    /// identifier declares-or-shadows in this context.
    pub(super) fn walk_pattern_declaration(&mut self, n: Node, scope: usize) {
        let mut targets = Vec::new();
        for child in pre_eq_nodes(n) {
            collect_identifiers(child, &mut targets);
        }
        for t in targets {
            let w = Write::assign(t.start_byte(), t.id(), None);
            self.bind_var(t, scope, w, IntroKind::Assign);
        }
        // the pattern side is fully consumed by the binder above;
        // walking it again would turn every target into its own reader
        // and kill NeverUsed reporting for destructured names
        walk_post_eq(self, n, scope);
    }

    /// `[a, b] = pair;`: every identifier on the pattern side becomes an
    /// assignment write; the RHS walks as ordinary expressions.
    pub(super) fn walk_pattern_assignment(&mut self, n: Node, scope: usize) {
        let mut targets = Vec::new();
        for child in pre_eq_nodes(n) {
            collect_identifiers(child, &mut targets);
        }
        for t in targets {
            if !self.rebind_local(t, scope, true, None) {
                // outer-state target: keep the operand read
                let name = self.text_of(t).to_string();
                self.model.record_read(scope, &name, t.start_byte());
            }
        }
        walk_post_eq(self, n, scope);
    }

    /// Plain `=` rebinds a visible local (one candidate write);
    /// compound operators rewrite-and-read. Targets that are no visible
    /// binding are fields or outer symbols: operand reads only.
    pub(super) fn walk_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain = n
            .child_by_field_name("operator")
            .and_then(|o| o.utf8_text(self.src).ok())
            == Some("=");
        if let Some(left) = left {
            if let Some(target) = bare_target(left) {
                if !self.rebind_local(target, scope, plain, right.map(|r| r.id())) {
                    // member/indexer targets (or shadowed outer state):
                    // operands are reads
                    self.walk_children(left, scope);
                }
            } else {
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }
}

/// Top-level children before the `=` of a pattern declaration/assignment
/// -- the pattern side proper.
fn pre_eq_nodes<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        if child.kind() == "=" && !child.is_named() {
            break;
        }
        out.push(child);
    }
    out
}

fn walk_post_eq(b: &mut Collector<'_>, n: Node, scope: usize) {
    let mut past_eq = false;
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        if !past_eq {
            if child.kind() == "=" && !child.is_named() {
                past_eq = true;
            }
            continue;
        }
        dispatch(b, child, scope);
    }
}

fn collect_identifiers<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    if n.kind() == "identifier" {
        out.push(n);
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        collect_identifiers(child, out);
    }
}

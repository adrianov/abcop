//! The ABC counter for the C-family backend: classifies tree-sitter nodes
//! into A(ssignment) / B(ranch) / C(ondition) buckets and walks unit bodies
//! without descending into nested unit roots.

use tree_sitter::Node;

use super::spec::{Spec, op_text};

const COMPARISONS: &[&str] = &["==", "===", "!=", "!==", "<", ">", "<=", ">=", "<=>"];
const LOGICAL: &[&str] = &["&&", "||", "??"];
/// Unary operators counted as branches; pointer `*`/`&` are memory access,
/// not computation, and stay uncounted.
const UNARY_B: &[&str] = &["!", "~", "-", "+"];

/// True when the operator token belongs to the branch-counted unary subset.
pub(super) fn unary_branch(op: Option<&str>) -> bool {
    op.is_some_and(|op| UNARY_B.contains(&op))
}

#[derive(Default)]
pub(super) struct Tally {
    a: u32,
    b: u32,
    c: u32,
}

impl Tally {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// A/B/C counts at scoring time, consumed once per unit.
    pub(super) fn counts(&self) -> (u32, u32, u32) {
        (self.a, self.b, self.c)
    }

    pub(super) fn walk(
        &mut self,
        spec: &Spec,
        n: Node,
        src: &[u8],
        roots: &std::collections::HashSet<usize>,
    ) {
        let children: Vec<_> = n.children(&mut n.walk()).collect();
        for child in children {
            if !roots.contains(&child.start_byte()) {
                self.walk(spec, child, src, roots);
            }
        }
        self.count(spec, n, src);
    }

    fn count(&mut self, spec: &Spec, n: Node, src: &[u8]) {
        if self.count_assignment(spec, n) || self.count_fixed_bucket(spec, n.kind()) {
            return;
        }
        self.count_operator_kinds(spec, n, src);
    }

    /// A: assignments; conditional forms only when they carry an initializer.
    fn count_assignment(&mut self, spec: &Spec, n: Node) -> bool {
        let k = n.kind();
        let counted = spec.assigns.contains(&k)
            && (!spec.conditional_assigns.contains(&k) || n.child_by_field_name("value").is_some());
        if counted {
            self.a += 1;
        }
        counted
    }

    /// Buckets encoded directly in the node kind: B calls, C conditions,
    /// B arithmetic-only operator kinds.
    fn count_fixed_bucket(&mut self, spec: &Spec, k: &str) -> bool {
        if spec.calls.contains(&k) {
            self.b += 1;
            return true;
        }
        if spec.conds.contains(&k) {
            self.c += 1;
            return true;
        }
        if spec.op_arith_kinds.contains(&k) {
            self.b += 1;
            return true;
        }
        false
    }

    /// Kinds sharing one node across operator flavors: binary (B vs C by
    /// token) and unary expressions (B for the counted subset).
    fn count_operator_kinds(&mut self, spec: &Spec, n: Node, src: &[u8]) {
        let k = n.kind();
        if k == spec.op_binary_kind {
            match op_text(n, src) {
                Some(op) if COMPARISONS.contains(&op) || LOGICAL.contains(&op) => self.c += 1,
                Some(_) => self.b += 1,
                None => {}
            }
            return;
        }
        if k == "unary_expression" && unary_branch(op_text(n, src)) {
            self.b += 1;
        }
    }
}

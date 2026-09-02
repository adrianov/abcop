//! AbcSize node counters, dispatched from `Calc::walk` (calc.rs).

use tree_sitter::Node;

use super::calc::Calc;
use super::helpers::{
    COMPARISON_OPS, is_non_send_callee, iterating_call, masgn_target_count, param_names,
};

impl<'f> Calc<'f> {
    pub(crate) fn count(&mut self, n: Node) {
        // anonymous keyword tokens share kind names with clause nodes
        if !n.is_named() {
            return;
        }
        let kind = n.kind();
        if Self::IS_ASSIGNMENT.contains(&kind) {
            return self.count_assignment(n);
        }
        if Self::IS_FLOW.contains(&kind) {
            return self.count_flow(n, kind);
        }
        self.count_calls(n);
    }

    fn count_assignment(&mut self, n: Node) {
        match n.kind() {
            "assignment" => self.count_assignment_node(n),
            "operator_assignment" => self.count_operator_assignment(n),
            "for" => {
                self.a += 1;
                self.c += 1;
            }
            _ => {}
        }
    }

    fn count_assignment_node(&mut self, n: Node) {
        match n.child_by_field_name("left") {
            Some(l)
                if l.kind() == "left_assignment_list" || l.kind() == "right_assignment_list" =>
            {
                self.a += masgn_target_count(self.fm, l);
            }
            Some(l) if l.kind() == "identifier" => self.count_lvasgn(l),
            _ => self.a += 1,
        }
    }

    fn count_lvasgn(&mut self, l: Node) {
        // resets the repeated-csend discount regardless of underscore naming
        let name = self.fm.text(l);
        self.seen_csend.remove(name);
        if !name.starts_with('_') {
            self.a += 1;
        }
    }

    fn count_operator_assignment(&mut self, n: Node) {
        let lhs = n
            .child_by_field_name("left")
            .map(|l| self.asgn_child_score(l, true));
        let rhs = n
            .child_by_field_name("right")
            .map(|r| self.asgn_child_score(r, false));
        self.a += lhs.unwrap_or(0) + rhs.unwrap_or(0);
        if matches!(self.field(n, "operator"), "||=" | "&&=") {
            self.c += 1;
        }
    }

    fn count_calls(&mut self, n: Node) {
        match n.kind() {
            "binary" => self.count_binary(n),
            "element_reference" | "yield" => self.b += 1,
            "unary" => self.count_unary(n),
            "identifier" => self.count_identifier(n),
            "call" => self.count_call(n),
            "block_argument" => self.count_block_argument(n),
            "block" | "do_block" => self.count_block(n),
            "lambda" | "method" | "singleton_method" => self.add_param_assignments(n),
            _ => {}
        }
    }

    fn count_binary(&mut self, n: Node) {
        let op = self.field(n, "operator");
        if COMPARISON_OPS.contains(&op) || op == "&&" || op == "||" {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }

    fn count_unary(&mut self, n: Node) {
        // `defined?` is neither branch nor condition. Signed numerics are
        // literals in parser/RuboCop but `unary`+operand here — fold them.
        let op = self.field(n, "operator");
        let folded_number = matches!(op, "-" | "+")
            && n.child_by_field_name("operand").is_some_and(|o| {
                matches!(o.kind(), "integer" | "float" | "rational" | "complex")
            });
        if op != "defined?" && !folded_number {
            self.b += 1;
        }
    }

    fn count_identifier(&mut self, n: Node) {
        // unresolved bare identifier == zero-arity method call -> branch
        if self.vcall.contains(&n.start_byte()) {
            self.b += 1;
        }
    }

    fn count_call(&mut self, n: Node) {
        let op = self.field(n, "operator").to_string();
        if is_non_send_callee(self.fm, n, &op) {
            return;
        }
        if op == "&." {
            self.count_safe_nav(n);
        } else {
            self.b += 1;
        }
    }

    fn count_safe_nav(&mut self, n: Node) {
        self.b += 1;
        let discounted = n
            .child_by_field_name("receiver")
            .and_then(|r| self.csend_recv.get(&r.start_byte()))
            .map(|name| !self.seen_csend.insert(name.clone()))
            .unwrap_or(false);
        if !discounted {
            self.c += 1;
        }
    }

    fn count_block_argument(&mut self, n: Node) {
        let call = n.parent().and_then(|al| al.parent());
        if call.is_some_and(|c| c.kind() == "call" && iterating_call(self.fm, c)) {
            self.c += 1;
        }
    }

    fn count_block(&mut self, n: Node) {
        let iterating = n
            .parent()
            .is_some_and(|p| p.kind() == "call" && iterating_call(self.fm, p));
        if iterating {
            self.c += 1;
        }
        self.add_param_assignments(n);
    }

    fn add_param_assignments(&mut self, unit: Node) {
        let Some(params) = unit.child_by_field_name("parameters") else {
            return;
        };
        self.a += param_names(self.fm, params)
            .into_iter()
            .filter(|nm| !nm.starts_with('_'))
            .count() as u32;
    }

    fn field(&self, n: Node, field: &str) -> &str {
        n.child_by_field_name(field)
            .map(|o| self.fm.text(o))
            .unwrap_or("")
    }
}

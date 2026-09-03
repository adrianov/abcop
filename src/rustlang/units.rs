//! AbcSize scoring for Rust function units.

use tree_sitter::Node;

use super::patterns::{match_binders, pattern_identifiers};
use super::scope::RustFile;
use super::skip_subtree;

const COMPARISON_OPS: &[&str] = &["==", "!=", "<", "<=", ">", ">="];

pub(super) struct Calc<'f> {
    fm: &'f RustFile<'f>,
    pub(super) a: u32,
    pub(super) b: u32,
    pub(super) c: u32,
}

fn pattern_count(fm: &RustFile, pattern: Node) -> u32 {
    let mut ids = Vec::new();
    pattern_identifiers(pattern, fm.src, &mut ids);
    ids.len() as u32
}

impl<'f> Calc<'f> {
    pub(super) fn over(fm: &'f RustFile) -> Self {
        Self {
            fm,
            a: 0,
            b: 0,
            c: 0,
        }
    }

    /// Post-order walk of the unit body, accumulating the ABC vector.
    pub(super) fn score(mut self, body: Node) -> (u32, u32, u32) {
        self.walk(body);
        (self.a, self.b, self.c)
    }

    fn walk(&mut self, n: Node) {
        let children: Vec<_> = n.children(&mut n.walk()).collect();
        for ch in children {
            self.walk(ch);
        }
        self.count(n);
    }

    fn count(&mut self, n: Node) {
        if !n.is_named() || skip_subtree(n.kind()) {
            return;
        }
        let kind = n.kind();
        if Self::DECL.contains(&kind) {
            return self.count_decl(n);
        }
        if Self::FLOW.contains(&kind) {
            return self.count_flow(n, kind);
        }
        if Self::OPS.contains(&kind) {
            self.count_ops(n)
        }
    }

    const DECL: [&'static str; 5] = [
        "let_declaration",
        "assignment_expression",
        "compound_assignment_expr",
        "closure_parameters",
        "parameters",
    ];

    const FLOW: [&'static str; 6] = [
        "for_expression",
        "if_let_expression",
        "while_let_expression",
        "match_arm",
        "if_expression",
        "while_expression",
    ];
    const OPS: [&'static str; 5] = [
        "binary_expression",
        "unary_expression",
        "call_expression",
        "macro_invocation",
        "try_expression",
    ];

    fn count_decl(&mut self, n: Node) {
        match n.kind() {
            "let_declaration" => {
                if let Some(p) = n.child_by_field_name("pattern") {
                    self.a += pattern_count(self.fm, p);
                }
            }
            "assignment_expression" | "compound_assignment_expr" => self.a += 1,
            "closure_parameters" | "parameters" => {
                self.a += Self::param_names(self.fm, n)
                    .into_iter()
                    .filter(|nm| !nm.starts_with('_'))
                    .count() as u32;
            }
            _ => {}
        }
    }

    fn count_flow(&mut self, n: Node, kind: &str) {
        match kind {
            "match_arm" => self.count_match_arm(n),
            "for_expression"
            | "if_let_expression"
            | "while_let_expression"
            | "if_expression"
            | "while_expression" => {
                self.c += 1;
                self.count_flow_pattern(n, kind);
            }
            _ => {}
        }
    }

    fn count_match_arm(&mut self, n: Node) {
        self.c += 1;
        if let Some(p) = n.child_by_field_name("pattern") {
            let mut binders = Vec::new();
            match_binders(p, self.fm.src, &mut binders);
            self.a += binders.len() as u32;
        }
    }

    /// `for`/`if let` bind their pattern identifiers on top of the branch.
    fn count_flow_pattern(&mut self, n: Node, kind: &str) {
        if !matches!(kind, "for_expression" | "if_let_expression") {
            return;
        }
        if let Some(p) = n.child_by_field_name("pattern") {
            self.a += pattern_count(self.fm, p);
        }
    }

    fn count_ops(&mut self, n: Node) {
        match n.kind() {
            "binary_expression" => self.count_binary(n),
            "unary_expression" => self.count_unary(n),
            // calls, macros and `?` are plain operations
            "call_expression" | "macro_invocation" | "try_expression" => self.b += 1,
            _ => {}
        }
    }

    fn count_binary(&mut self, n: Node) {
        let op = n
            .child_by_field_name("operator")
            .map(|o| self.fm.text(o))
            .unwrap_or("");
        if COMPARISON_OPS.contains(&op) || op == "&&" || op == "||" {
            self.c += 1;
        } else {
            self.b += 1;
        }
    }

    fn count_unary(&mut self, n: Node) {
        // `-1` folds into the literal; other unaries are operations
        let numeric_fold = n
            .child_by_field_name("operator")
            .map(|o| matches!(self.fm.text(o), "-" | "+"))
            .unwrap_or(false)
            && n.child_by_field_name("operand")
                .map(|o| matches!(o.kind(), "integer_literal" | "float_literal"))
                .unwrap_or(false);
        if !numeric_fold {
            self.b += 1;
        }
    }

    fn param_names<'t>(fm: &'t RustFile, container: Node) -> Vec<&'t str> {
        let mut out = Vec::new();
        let mut cursor = container.walk();
        for child in container.children(&mut cursor) {
            match child.kind() {
                "identifier" => out.push(fm.text(child)),
                "parameter" => {
                    if let Some(pat) = child.child_by_field_name("pattern") {
                        let mut ids = Vec::new();
                        pattern_identifiers(pat, fm.src, &mut ids);
                        for id in ids {
                            out.push(fm.text(id));
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

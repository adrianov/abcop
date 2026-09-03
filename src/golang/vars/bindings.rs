//! Write binding for assignment-shaped nodes: plain `=` / `:=`,
//! compound assignments, inc/dec statements, var specs and range clauses.

use tree_sitter::Node;

use super::collector::Collector;
use super::{IntroKind, Write};

impl Collector<'_> {
    /// Dispatch every kind that carries variable writes or a bare
    /// identifier read. Returns false for anything else, leaving the
    /// scope walker free to recurse generically.
    pub(super) fn bind_or_read(&mut self, n: Node, scope: usize) -> bool {
        match n.kind() {
            "short_var_declaration" | "assignment_statement" => {
                self.bind_plain_or_compound(n, scope);
                self.walk_right(n, scope);
            }
            "var_spec" => self.bind_var_spec(n, scope),
            "inc_statement" | "dec_statement" => {
                // i++ / i-- : reads and rewrites
                self.bind_inc_dec(n, scope);
            }
            // range variables are loop protocol, never tracked
            "range_clause" => self.bind_range_clause(n, scope),
            "identifier" => self.record_identifier_read(n, scope),
            _ => return false,
        }
        true
    }

    /// Plain `=` vs compound: distinguishable by operator token.
    fn bind_plain_or_compound(&mut self, n: Node, scope: usize) {
        if n.kind() == "short_var_declaration" || self.first_anon_op(n) == Some("=") {
            self.bind_assignment(n, scope);
        } else {
            self.bind_compound(n, scope);
        }
    }

    fn walk_right(&mut self, n: Node, scope: usize) {
        if let Some(right) = n.child_by_field_name("right") {
            self.walk(right, scope);
        }
    }

    /// Plain `=` / `:=`: each expression-list element either binds an
    /// identifier or is a reference whose operands become reads.
    fn bind_assignment(&mut self, n: Node, scope: usize) {
        let Some(left) = n.child_by_field_name("left") else {
            return;
        };
        self.bind_targets(left, n.child_by_field_name("right"), scope);
        // reference-style elements (t.n = ..., m[k] = ...): operand reads
        for element in left.children(&mut left.walk()) {
            if element.kind() != "identifier" {
                self.walk(element, scope);
            }
        }
    }

    /// Bind each identifier target; a lone target links its write to
    /// the single top-level value of the expression list.
    fn bind_targets(&mut self, left: Node, right: Option<Node>, scope: usize) {
        let mut targets = Vec::new();
        Self::identifier_targets(left, &mut targets);
        let single = targets.len() == 1;
        for t in targets {
            let rhs = if single {
                right.and_then(|r| r.named_child(0)).map(|v| v.id())
            } else {
                None
            };
            self.write_plain(t, rhs, scope);
        }
    }

    /// Bind one plain write positioned at identifier `t`, optionally
    /// linked to its RHS node id.
    fn write_plain(&mut self, t: Node, rhs: Option<usize>, scope: usize) {
        let w = Write {
            byte: t.start_byte(),
            node_id: t.id(),
            plain: true,
            rhs,
        };

        self.bind(scope, &self.text(t).to_string(), w, IntroKind::Assign);
    }

    /// Identifier targets nested inside an expression list element.
    fn identifier_targets<'t>(left: Node<'t>, out: &mut Vec<Node<'t>>) {
        if left.kind() == "identifier" {
            out.push(left);
            return;
        }
        let mut cursor = left.walk();
        for child in left.children(&mut cursor) {
            Self::identifier_targets(child, out);
        }
    }

    /// Bind a compound-style write (plain=false) plus its implied read.
    fn bind_rewrite(&mut self, t: Node, scope: usize) {
        let byte = t.start_byte();
        let w = Write {
            byte,
            node_id: t.id(),
            plain: false,
            rhs: None,
        };
        let name = self.text(t).to_string();
        self.bind(scope, &name, w, IntroKind::Binding);
        self.record_read(scope, &name, byte + 1);
    }

    fn bind_inc_dec(&mut self, n: Node, scope: usize) {
        let children: Vec<_> = n.children(&mut n.walk()).collect();
        for child in children {
            if child.kind() == "identifier" {
                self.bind_rewrite(child, scope);
            } else {
                self.walk(child, scope);
            }
        }
    }

    /// Range variables are loop protocol, never tracked: skip the left
    /// side, walk the rest.
    fn bind_range_clause(&mut self, n: Node, scope: usize) {
        let skipped = n.child_by_field_name("left").map(|l| l.id());
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if Some(child.id()) != skipped {
                self.walk(child, scope);
            }
        }
    }

    fn bind_compound(&mut self, n: Node, scope: usize) {
        if let Some(left) = n.child_by_field_name("left") {
            let mut targets = Vec::new();
            Self::identifier_targets(left, &mut targets);
            for t in targets {
                self.bind_rewrite(t, scope);
            }
        }
    }

    fn record_identifier_read(&mut self, n: Node, scope: usize) {
        self.record_read(scope, &self.text(n).to_string(), n.start_byte());
    }

    /// Split a var spec into declared identifier names before the `=`
    /// token and value expressions after it.
    fn split_var_spec<'t>(&self, n: Node<'t>) -> (Vec<Node<'t>>, Vec<Node<'t>>) {
        let mut names = Vec::new();
        let mut values = Vec::new();
        let mut past_eq = false;
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if !child.is_named() && self.text(child) == "=" {
                past_eq = true;
            } else if child.is_named() && child.kind() == "identifier" && !past_eq {
                names.push(child);
            } else if past_eq {
                values.push(child);
            }
        }
        (names, values)
    }

    /// `var u, w = v, *p`: declared names precede the `=` token; values
    /// follow. RHS links only when one name maps to one value.
    fn bind_var_spec(&mut self, n: Node, scope: usize) {
        let (names, values) = self.split_var_spec(n);
        let single_pair = names.len() == 1 && values.len() == 1;
        for (idx, t) in names.iter().enumerate() {
            let rhs = if single_pair {
                values[0].id().into()
            } else {
                values.get(idx).map(|v| v.id())
            };
            self.write_plain(*t, rhs, scope);
        }
        for v in values {
            self.walk(v, scope);
        }
    }
}

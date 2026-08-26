//! Control-flow condition counters (`if`/`rescue`/modifier forms),
//! dispatched from the `Calc::count` kind dispatch.

use tree_sitter::Node;

use super::calc::Calc;
use crate::model::FileModel;

impl<'f> Calc<'f> {
    pub(crate) fn count_flow(&mut self, n: Node, kind: &str) {
        match kind {
            "if" | "unless" | "elsif" => self.count_if(n),
            "rescue" | "rescue_modifier" => self.count_rescue(n, kind),
            "if_modifier" | "unless_modifier" | "conditional" | "when" | "in_clause" | "while"
            | "until" | "while_modifier" | "until_modifier" => self.c += 1,
            _ => {}
        }
    }

    fn count_if(&mut self, n: Node) {
        self.c += 1;
        let has_else = {
            let mut cur = n.walk();
            n.children(&mut cur).any(|ch| ch.kind() == "else")
        };
        if has_else {
            self.c += 1;
        }
    }

    fn count_rescue(&mut self, n: Node, kind: &str) {
        // a multi-clause rescue group is ONE :rescue node; TS emits siblings
        let first_clause = n
            .prev_named_sibling()
            .map(|p| p.kind() != "rescue")
            .unwrap_or(true);
        if kind == "rescue_modifier" || first_clause {
            self.c += 1;
        }
        if rescue_binds_named_variable(self.fm, n) {
            self.a += 1;
        }
    }
}

/// A rescue variable binds a usable local when it names a non-underscore identifier.
fn rescue_binds_named_variable(fm: &FileModel, n: Node) -> bool {
    n.child_by_field_name("variable").is_some_and(|var| {
        var.children(&mut var.walk())
            .any(|c| c.kind() == "identifier" && !fm.text(c).starts_with('_'))
    })
}

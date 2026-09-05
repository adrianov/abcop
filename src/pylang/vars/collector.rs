//! Scope-tree construction and per-scope identifier bookkeeping.

mod dispatch;

use std::collections::HashMap;

use tree_sitter::Node;

use super::{Entry, IntroKind, Scope, ScopeKind, Write};

/// Build the scope tree and per-identifier bookkeeping for a parsed file.
pub(crate) fn collect(root: Node, src: &[u8]) -> Vec<Scope> {
    let mut c = Collector {
        src,
        scopes: vec![Scope {
            parent: None,
            kind: ScopeKind::Root,
            entries: HashMap::new(),
        }],
    };
    c.walk(root, 0);
    c.scopes
}

struct Collector<'a> {
    src: &'a [u8],
    scopes: Vec<Scope>,
}

impl Collector<'_> {
    fn open_scope(&mut self, kind: ScopeKind, parent: usize) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    /// Name exists in this scope (Python locals are scope-wide, so a
    /// same-scope name always shadows outer scopes regardless of
    /// position); the read only counts when the binding was already
    /// introduced at the read position.
    fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let data = &self.scopes[scope];
        if let Some(e) = data.entries.get(name) {
            return if e.intro_byte <= pos {
                Some(scope)
            } else {
                None
            };
        }
        match data.kind {
            ScopeKind::Block => self.lookup(data.parent?, pos, name),
            _ => None,
        }
    }

    fn bind(&mut self, scope: usize, name: &str, w: Write, intro: IntroKind) {
        if name.starts_with('_') {
            return;
        }
        match self.lookup(scope, w.byte, name) {
            Some(s) => {
                self.scopes[s]
                    .entries
                    .get_mut(name)
                    .expect("looked-up entry")
                    .writes
                    .push(w);
            }
            None => {
                self.scopes[scope].entries.insert(
                    Box::from(name),
                    Entry {
                        intro_byte: w.byte,
                        intro_kind: intro,
                        writes: vec![w],
                        reads: Vec::new(),
                    },
                );
            }
        }
    }

    fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
        if name.starts_with('_') {
            return;
        }
        if let Some(s) = self.lookup(scope, byte, name) {
            self.scopes[s]
                .entries
                .get_mut(name)
                .expect("looked-up entry")
                .reads
                .push(byte);
        }
    }

    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    /// Bind every name *written* by an assignment target expression.
    /// Pattern lists expand per bound name; attribute (`obj.attr =`) and
    /// subscript (`obj[k] =`) targets merely reference their operands --
    /// those operands become ordinary reads instead.
    fn bind_targets(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "identifier" => self.bind_name(n, scope, IntroKind::Assign, None),
            // expand pattern lists per bound name; must NOT recurse
            // through walk(), which would treat names as reads
            "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat"
            | "dictionary_splat" => {
                let children: Vec<_> = n.children(&mut n.walk()).collect();
                for child in children {
                    self.bind_targets(child, scope);
                }
            }
            // reference-style targets (obj.attr / obj[k]): operands are reads
            _ => self.walk_children(n, scope),
        }
    }

    /// Track an `as <name>` protocol binding (with/except/match aliases)
    /// as a non-candidate write.
    fn bind_alias(&mut self, n: Node, scope: usize) {
        if n.kind() == "identifier" {
            self.bind_name(n, scope, IntroKind::Binding, None);
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.bind_alias(child, scope);
        }
    }

    /// Bind every identifier inside a match `case` pattern as a capture
    /// (never an inline candidate); same leaf-binding traversal as alias
    /// handling, entered from case patterns.
    fn bind_captures(&mut self, n: Node, scope: usize) {
        self.bind_alias(n, scope);
    }

    /// Bind a written identifier token under `intro`: assignment/walrus
    /// heads pass an RHS node id, protocol bindings and pattern captures
    /// none -- only plain Assign writes are inline candidates. Module and
    /// class-body names are attributes/exports (not locals) and stay
    /// unbound; callers still walk the RHS for nested locals.
    fn bind_name(&mut self, name_node: Node, scope: usize, intro: IntroKind, rhs: Option<usize>) {
        if matches!(
            self.scopes[scope].kind,
            ScopeKind::Root | ScopeKind::Class
        ) {
            return;
        }
        let w = Write {
            byte: name_node.start_byte(),
            node_id: name_node.id(),
            plain: intro == IntroKind::Assign,
            rhs,
        };

        self.bind(scope, &self.text(name_node).to_string(), w, intro);
    }
}

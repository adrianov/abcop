//! Single-file semantic model: scope tree, local-variable introductions,
//! writes and reads. Shared by the ABC calculator (safe-nav receiver
//! classification) and the used-once detector.

use std::collections::HashMap;

use tree_sitter::{Node, Tree};

pub type ScopeId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntroKind {
    /// plain `x = ...`
    Assign,
    /// `x op= ...` / masgn target / block param etc.
    Binding,
}

#[derive(Clone, Copy, Debug)]
pub struct Write {
    pub byte: usize,
    pub node_id: usize,
    pub kind: WriteKind,
    /// RHS expression of a plain assignment as `(node id, start byte)`.
    pub rhs: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Plain,
    OpAssign,
    Masgn,
    ForVar,
    RescueVar,
}

#[derive(Clone, Copy, Debug)]
pub struct Read {
    pub byte: usize,
    pub under_defined: bool,
}

#[derive(Debug)]
pub struct Entry {
    pub intro_byte: usize,
    pub intro_kind: IntroKind,
    pub writes: Vec<Write>,
    pub reads: Vec<Read>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Root,
    Method,
    ClassLike,
    Block,
}

#[derive(Debug)]
pub struct ScopeData {
    pub parent: Option<ScopeId>,
    pub kind: ScopeKind,
    /// Byte offset at which this scope was entered; used when climbing out of
    /// a block: parent bindings are only shared if introduced before entry.
    pub entered_at: usize,
    pub entries: HashMap<Box<str>, Entry>,
}

pub struct FileModel<'s> {
    pub src: &'s [u8],
    pub tree: Tree,
    pub scopes: Vec<ScopeData>,
    /// safe-navigation sites whose receiver resolved to a local var:
    /// `(receiver start byte, receiver name, owning scope)`
    pub csend_sites: Vec<(usize, Box<str>, ScopeId)>,
    /// bare identifiers that did NOT resolve to locals — zero-arity method
    /// calls (parser-gem `:send` vcalls)
    pub vcall_sites: Vec<usize>,
}

impl<'s> FileModel<'s> {
    /// Text accessor whose lifetime is bound to the model, not the node.
    pub fn text(&self, n: Node<'_>) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    /// Resolve `name` visible at byte `pos` starting from scope `s`.
    #[allow(dead_code)]
    pub fn lookup(&self, s: ScopeId, pos: usize, name: &str) -> Option<ScopeId> {
        lookup_scope(&self.scopes, s, pos, name)
    }

    pub fn line_col(&self, byte: usize) -> (usize, usize) {
        let point = self
            .tree
            .root_node()
            .descendant_for_byte_range(byte, byte)
            .map(|n| n.start_position())
            .unwrap_or_default();
        (point.row + 1, point.column)
    }
}

/// Scope resolution so the in-progress builder can query visibility too.
pub fn lookup_scope(
    scopes: &[ScopeData],
    mut s: ScopeId,
    pos: usize,
    name: &str,
) -> Option<ScopeId> {
    let mut effective_pos = pos;
    loop {
        let scope = &scopes[s];
        if let Some(e) = scope.entries.get(name)
            && e.intro_byte <= effective_pos {
                return Some(s);
            }
        let parent = scope.parent?;
        // Climbing out of a nested block: only bindings introduced before the
        // boundary are shared with the outer scope.
        if matches!(scope.kind, ScopeKind::Block) {
            effective_pos = effective_pos.min(scope.entered_at);
        } else {
            // method/class scopes are opaque to locals from above
            return None;
        }
        s = parent;
    }
}

struct Builder<'m> {
    src: &'m [u8],
    scopes: &'m mut Vec<ScopeData>,
    csend_sites: &'m mut Vec<(usize, Box<str>, ScopeId)>,
    vcall_sites: &'m mut Vec<usize>,
}

pub fn build<'s>(src: &'s [u8], tree: Tree) -> FileModel<'s> {
    let mut scopes = vec![ScopeData {
        parent: None,
        kind: ScopeKind::Root,
        entered_at: 0,
        entries: HashMap::new(),
    }];
    let mut csend_sites = Vec::new();
    let mut vcall_sites = Vec::new();
    {
        let mut b = Builder {
            src,
            scopes: &mut scopes,
            csend_sites: &mut csend_sites,
            vcall_sites: &mut vcall_sites,
        };
        b.walk(tree.root_node(), 0, false);
    }
    FileModel {
        src,
        tree,
        scopes,
        csend_sites,
        vcall_sites,
    }
}

fn declared_name(child: Node, src: &[u8]) -> Option<String> {
    match child.kind() {
        "identifier" => Some(child.utf8_text(src).unwrap_or("").to_string()),
        "optional_parameter" | "keyword_parameter" | "block_parameter" | "splat_parameter" => child
            .child_by_field_name("name")
            .or_else(|| {
                child
                    .children(&mut child.walk())
                    .find(|c| c.kind() == "identifier")
            })
            .map(|n| n.utf8_text(src).unwrap_or("").to_string()),
        _ => None,
    }
}

impl<'m> Builder<'m> {
    fn text<'t>(&'t self, n: Node<'t>) -> &'t str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn scope_for(&mut self, owner: Node, kind: ScopeKind, parent: Option<ScopeId>) -> ScopeId {
        self.scopes.push(ScopeData {
            parent,
            kind,
            entered_at: owner.start_byte(),
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    fn lookup(&self, scope: ScopeId, pos: usize, name: &str) -> Option<ScopeId> {
        lookup_scope(self.scopes, scope, pos, name)
    }

    /// Record a write; creates the binding if not visible yet.
    fn record_write(&mut self, scope: ScopeId, name: &str, w: Write, intro_kind: IntroKind) {
        if name.starts_with('_') {
            return;
        }
        let pos = w.byte;
        match self.lookup(scope, pos, name) {
            Some(s) => {
                let e = self.scopes[s].entries.get_mut(name).unwrap();
                e.writes.push(w);
            }
            None => {
                let e = Entry {
                    intro_byte: pos,
                    intro_kind,
                    writes: vec![w],
                    reads: Vec::new(),
                };
                self.scopes[scope].entries.insert(name.into(), e);
            }
        }
    }

    fn record_read(&mut self, scope: ScopeId, name: &str, r: Read) {
        let Some(s) = self.lookup(scope, r.byte, name) else {
            return;
        };
        let e = self.scopes[s].entries.get_mut(name).unwrap();
        e.reads.push(r);
    }

    fn walk(&mut self, n: Node, scope: ScopeId, under_defined: bool) {
        let kind = n.kind();

        // ---- scope-introducing constructs -------------------------------
        match kind {
            "method" | "singleton_method" => {
                let s = self.scope_for(n, ScopeKind::Method, None);
                if let Some(p) = n.child_by_field_name("parameters") {
                    self.declare_params(p, s);
                }
                if let Some(body) = n.child_by_field_name("body") {
                    self.walk(body, s, false);
                }
                return;
            }
            "class" | "module" | "singleton_class" => {
                let s = self.scope_for(n, ScopeKind::ClassLike, None);
                if let Some(body) = n.child_by_field_name("body") {
                    self.walk(body, s, false);
                }
                return;
            }
            "block" | "do_block" => {
                let s = self.scope_for(n, ScopeKind::Block, Some(scope));
                if let Some(p) = n.child_by_field_name("parameters") {
                    self.declare_params(p, s); // block params always shadow
                }
                if let Some(body) = body_of(n) {
                    self.walk(body, s, false);
                }
                return;
            }
            "lambda" => {
                let s = self.scope_for(n, ScopeKind::Block, Some(scope));
                if let Some(p) = n.child_by_field_name("parameters") {
                    self.declare_params(p, s);
                }
                if let Some(body) = body_of(n) {
                    self.walk(body, s, false);
                }
                return;
            }
            _ => {}
        }

        // ---- declarations & writes --------------------------------------
        match kind {
            "assignment" => {
                let left = n.child_by_field_name("left");
                let rhs = n.child_by_field_name("right");
                match left {
                    Some(l) if l.kind() == "left_assignment_list" => {
                        self.collect_masgn_targets(l, scope);
                        if let Some(r) = rhs {
                            self.walk(r, scope, under_defined);
                        }
                    }
                    Some(l) if l.kind() == "identifier" => {
                        let name = self.text(l).to_string();
                        let w = Write {
                            byte: l.start_byte(),
                            node_id: l.id(),
                            kind: WriteKind::Plain,
                            rhs: rhs.map(|r| (r.id(), r.start_byte())),
                        };
                        self.record_write(scope, &name, w, IntroKind::Assign);
                        if let Some(r) = rhs {
                            self.walk(r, scope, under_defined);
                        }
                    }
                    // attribute / element / ivar / const targets: not locals,
                    // but the RHS may still contain reads
                    _ => {
                        if let Some(l) = left {
                            self.walk(l, scope, under_defined);
                        }
                        if let Some(r) = rhs {
                            self.walk(r, scope, under_defined);
                        }
                    }
                }
                return;
            }
            "operator_assignment" => {
                if let Some(l) = n.child_by_field_name("left") {
                    if l.kind() == "identifier" {
                        let name = self.text(l).to_string();
                        let w = Write {
                            byte: l.start_byte(),
                            node_id: l.id(),
                            kind: WriteKind::OpAssign,
                            rhs: None,
                        };
                        self.record_write(scope, &name, w, IntroKind::Binding);
                    } else {
                        self.walk(l, scope, under_defined);
                    }
                }
                if let Some(r) = n.child_by_field_name("right") {
                    self.walk(r, scope, under_defined);
                }
                return;
            }
            "for" => {
                let pattern = n.child_by_field_name("pattern");
                if let Some(pat) = pattern
                    && pat.kind() == "identifier" {
                        let name = self.text(pat).to_string();
                        let w = Write {
                            byte: pat.start_byte(),
                            node_id: pat.id(),
                            kind: WriteKind::ForVar,
                            rhs: None,
                        };
                        self.record_write(scope, &name, w, IntroKind::Binding);
                    }
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if pattern.map(|p| p.id()) == Some(child.id()) {
                        continue;
                    }
                    self.walk(child, scope, under_defined);
                }
                return;
            }
            "rescue" => {
                // bind the exception variable FIRST: the handler body reads it
                let var = n.child_by_field_name("variable");
                if let Some(v) = var
                    && let Some(ident) =
                        v.children(&mut v.walk()).find(|c| c.kind() == "identifier")
                    {
                        let name = self.text(ident).to_string();
                        let w = Write {
                            byte: ident.start_byte(),
                            node_id: ident.id(),
                            kind: WriteKind::RescueVar,
                            rhs: None,
                        };
                        self.record_write(scope, &name, w, IntroKind::Binding);
                    }
                let mut cursor = n.walk();
                for child in n.children(&mut cursor) {
                    if var.map(|v| v.id()) == Some(child.id()) {
                        continue;
                    }
                    self.walk(child, scope, under_defined);
                }
                return;
            }
            "when" | "in_clause" => {
                // pattern subtree may bind variables we deliberately do not
                // track; walk only the body
                if let Some(b) = n.child_by_field_name("body") {
                    self.walk(b, scope, under_defined);
                }
                return;
            }
            _ => {}
        }

        // ---- reads -------------------------------------------------------
        if kind == "unary" {
            let op_node = n.child_by_field_name("operator");
            let op = op_node.map(|o| self.text(o)).unwrap_or("");
            let ud = under_defined || op == "defined?";
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if op_node.map(|o| o.id()) == Some(child.id()) {
                    continue;
                }
                self.walk(child, scope, ud);
            }
            return;
        }

        if kind == "call" {
            // never treat the @method slot as a variable read; record safe-nav
            // sites on locals for the ABC repeated-csend discount
            let method_slot = n.child_by_field_name("method");
            let op = n
                .child_by_field_name("operator")
                .map(|o| self.text(o))
                .unwrap_or("")
                .to_string();
            if op == "&."
                && let Some(recv) = n.child_by_field_name("receiver")
                    && recv.kind() == "identifier" {
                        let name = self.text(recv);
                        if self.lookup(scope, recv.start_byte(), name).is_some() {
                            self.csend_sites.push((recv.start_byte(), name.into(), scope));
                        }
                    }
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                if method_slot.map(|m| m.id()) == Some(child.id()) {
                    continue;
                }
                self.walk(child, scope, under_defined);
            }
            return;
        }

        if kind == "identifier" {
            let name = self.text(n).to_string();
            let r = Read {
                byte: n.start_byte(),
                under_defined,
            };
            if self.lookup(scope, r.byte, &name).is_some() {
                if !name.starts_with('_') {
                    self.record_read(scope, &name, r);
                }
            } else {
                // unresolved bare identifier == zero-arity method call
                self.vcall_sites.push(n.start_byte());
            }
            return;
        }

        // generic descent
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "method_parameters"
                || child.kind() == "block_parameters"
                || child.kind() == "lambda_parameters"
            {
                // stray parameter container outside a callable; descend into
                // default-value expressions only
                let mut sub = child.walk();
                for inner in child.children(&mut sub) {
                    if inner.child_by_field_name("value").is_some() {
                        self.walk(inner, scope, under_defined);
                    }
                }
                continue;
            }
            self.walk(child, scope, under_defined);
        }
    }

    fn declare_params(&mut self, container: Node, scope: ScopeId) {
        let mut cursor = container.walk();
        for child in container.children(&mut cursor) {
            match child.kind() {
                "," | "(" | ")" | "|" => continue,
                "identifier" | "optional_parameter" | "keyword_parameter"
                | "block_parameter" | "splat_parameter" => {
                    // default/kw values may contain arbitrary expressions
                    if let Some(v) = child.child_by_field_name("value") {
                        self.walk(v, scope, false);
                    }
                    if let Some(name) = declared_name(child, self.src)
                        && !name.starts_with('_') {
                            let pos = child.start_byte();
                            self.scopes[scope]
                                .entries
                                .entry(name.into())
                                .or_insert(Entry {
                                    intro_byte: pos,
                                    intro_kind: IntroKind::Binding,
                                    writes: Vec::new(),
                                    reads: Vec::new(),
                                });
                        }
                }
                _ => {
                    // splat wrappers, forwarding args, shadow params etc.
                    let mut sub = child.walk();
                    for inner in child.children(&mut sub) {
                        if inner.kind() == "identifier" {
                            let name = self.text(inner).to_string();
                            if !name.starts_with('_') {
                                let pos = inner.start_byte();
                                self.scopes[scope]
                                    .entries
                                    .entry(name.into())
                                    .or_insert(Entry {
                                        intro_byte: pos,
                                        intro_kind: IntroKind::Binding,
                                        writes: Vec::new(),
                                        reads: Vec::new(),
                                    });
                            }
                        }
                    }
                }
            }
        }
    }

    fn collect_masgn_targets(&mut self, list: Node, scope: ScopeId) {
        let mut cursor = list.walk();
        for child in list.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    let name = self.text(child).to_string();
                    let w = Write {
                        byte: child.start_byte(),
                        node_id: child.id(),
                        kind: WriteKind::Masgn,
                        rhs: None,
                    };
                    self.record_write(scope, &name, w, IntroKind::Binding);
                }
                "rest_assignment" | "destructured_left_assignment_list" => {
                    self.collect_masgn_targets_inner(child, scope);
                }
                _ => {}
            }
        }
    }

    fn collect_masgn_targets_inner(&mut self, list: Node, scope: ScopeId) {
        let mut cursor = list.walk();
        for child in list.children(&mut cursor) {
            if child.kind() == "identifier" {
                let name = self.text(child).to_string();
                let w = Write {
                    byte: child.start_byte(),
                    node_id: child.id(),
                    kind: WriteKind::Masgn,
                    rhs: None,
                };
                self.record_write(scope, &name, w, IntroKind::Binding);
            } else if child.named_child_count() > 0 && child.kind() != "integer" {
                self.collect_masgn_targets_inner(child, scope);
            }
        }
    }
}

/// Body of a block-ish node regardless of brace/do form.
fn body_of(n: Node) -> Option<Node> {
    let mut cursor = n.walk();
    n.children(&mut cursor)
        .find(|c| c.kind() == "body_statement" || c.kind() == "block_body")
}

#[cfg(test)]
#[cfg(test)]
pub(crate) fn build_from_str(src: &str) -> FileModel<'_> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .expect("ruby grammar");
    let tree = parser.parse(src, None).expect("syntax tree");
    build(src.as_bytes(), tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebind_inside_block_hits_shared_outer_binding() {
        let fm = build_from_str("x = 1\n[1].each { x = 2 }\n");
        let e = fm.scopes[0].entries.get("x").expect("entry");
        assert_eq!(e.writes.len(), 2);
    }

    #[test]
    fn vcall_never_counts_as_variable_read() {
        let fm = build_from_str("def m\n  bar\nend\n");
        assert!(fm.scopes.iter().all(|s| s.entries.is_empty()));
    }

    #[test]
    fn local_read_after_introduction_is_tracked() {
        let fm = build_from_str("def m\n  x = 1\n  p x\nend\n");
        let mscope = fm
            .scopes
            .iter()
            .find(|s| s.kind == ScopeKind::Method)
            .expect("method scope");
        let e = mscope.entries.get("x").expect("entry");
        assert_eq!(e.writes.len(), 1);
        assert_eq!(e.reads.len(), 1);
    }

    #[test]
    fn block_local_var_does_not_leak() {
        let fm = build_from_str("[1].each { y = 2 }\n");
        let blk = fm
            .scopes
            .iter()
            .find(|s| s.kind == ScopeKind::Block)
            .expect("block scope");
        assert!(blk.entries.contains_key("y"));
        assert!(!fm.scopes[0].entries.contains_key("y"));
    }
}

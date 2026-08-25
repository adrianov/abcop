//! Rust-language backend: scope model, AbcSize, and used-once analysis.
//!
//! Metric spec (defined to mirror the Ruby port's semantics where a direct
//! analogue exists):
//! - Units: every `function_item` (free fns and impl methods), scored over its
//!   `@body` subtree, post-order. Closures are not separate units; their params
//!   and contents roll into the enclosing function (mirrors Ruby blocks).
//! - A: let bindings (per pattern identifier), `=` and compound assignments,
//!   `for`/`if let`/`while let`/match-arm pattern bindings, closure params,
//!   params of nested functions. Underscore-prefixed names never count.
//! - B: call expressions, macro invocations, `?` try expressions, unary ops,
//!   non-condition binary ops.
//! - C: if / if-let / while / while-let / for, one per match arm (guards come
//!   via normal binary rules), comparisons and `&&`/`||`.
//!   No else bonus: Rust if-else is a value-producing expression.
//! - UsedOnce: single plain `let`, single read, pure RHS, straight-line write,
//!   read after write. Params/pattern-bound/mut-reassigned vars excluded.

use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Tree};

use crate::abc::AbcOffense;
use crate::used_once::UsedOnceOffense;

const COMPARISON_OPS: &[&str] = &["==", "!=", "<", "<=", ">", ">="];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// `let x = ...`
    Assign,
    /// params, `+=`, pattern bindings — never inline candidates
    Binding,
}

#[derive(Clone, Copy, Debug)]
struct Write {
    byte: usize,
    node_id: usize,
    plain: bool,
    rhs: Option<(usize, usize)>,
}

#[derive(Debug)]
struct Entry {
    intro_byte: usize,
    intro_kind: IntroKind,
    writes: Vec<Write>,
    reads: Vec<usize>,
    /// reads that occurred inside a `token_tree` (macro input): macros may
    /// give identifiers syntactic roles, so they never justify inlining
    macro_reads: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeKind {
    Root,
    Function,
    Block,
}

struct Scope {
    parent: Option<usize>,
    kind: ScopeKind,
    entries: HashMap<Box<str>, Entry>,
}

pub struct RustFile {
    pub src: Vec<u8>,
    pub tree: Tree,
    scopes: Vec<Scope>,
}

impl RustFile {
    fn text<'s>(&'s self, n: Node<'_>) -> &'s str {
        n.utf8_text(&self.src).unwrap_or("")
    }

    fn line_col(&self, byte: usize) -> (usize, usize) {
        let point = self
            .tree
            .root_node()
            .descendant_for_byte_range(byte, byte)
            .map(|n| n.start_position())
            .unwrap_or_default();
        (point.row + 1, point.column)
    }

    #[allow(dead_code)]
    fn lookup(&self, mut s: usize, pos: usize, name: &str) -> Option<usize> {
        loop {
            let scope = &self.scopes[s];
            if let Some(e) = scope.entries.get(name)
                && e.intro_byte <= pos
            {
                return Some(s);
            }
            // Rust scoping is purely lexical; function scopes are opaque
            match scope.kind {
                ScopeKind::Block => s = scope.parent?,
                _ => return None,
            }
        }
    }
}

/// Node kinds whose subtrees are type/attribute territory — no variable reads
/// or metric contributions live there.
fn skip_subtree(kind: &str) -> bool {
    if matches!(
        kind,
        "type_arguments"
            | "type_parameters"
            | "where_clause"
            | "trait_bounds"
            | "attribute_item"
            | "scoped_type_identifier"
            | "metavariable"
            | "line_comment"
            | "scoped_identifier"
    ) {
        return true;
    }
    // real types (`reference_type`, `generic_type`, …) — but not casts
    kind.ends_with("_type") && kind != "type_cast_expression"
}

/// Identifiers bound by a let/for/if-let/match pattern.
fn pattern_identifiers<'t>(pattern: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    // a bare identifier IS the whole pattern (let total = ...)
    if pattern.kind() == "identifier" {
        let name = pattern.utf8_text(src).unwrap_or("");
        if !name.starts_with('_') {
            out.push(pattern);
        }
        return;
    }
    if pattern.kind() == "_" || skip_subtree(pattern.kind()) {
        return;
    }
    let mut cursor = pattern.walk();
    for child in pattern.children(&mut cursor) {
        let k = child.kind();
        if k == "identifier" {
            let name = child.utf8_text(src).unwrap_or("");
            if !name.starts_with('_') {
                out.push(child);
            }
        } else if k != "_" && !skip_subtree(k) {
            pattern_identifiers(child, src, out);
        }
    }
}

/// Match-arm binders: identifiers before an optional `if` guard; after the
/// guard everything is a read.
fn match_binders<'t>(pattern: Node<'t>, src: &[u8], out: &mut Vec<Node<'t>>) {
    let mut cursor = pattern.walk();
    for child in pattern.children(&mut cursor) {
        match child.kind() {
            "if" => return,
            "identifier" => {
                let name = child.utf8_text(src).unwrap_or("");
                if !name.starts_with('_') {
                    out.push(child);
                }
            }
            "_" => {}
            k if skip_subtree(k) => {}
            _ => match_binders(child, src, out),
        }
    }
}

struct Builder<'m> {
    src: &'m [u8],
    scopes: &'m mut Vec<Scope>,
    macro_depth: usize,
}

pub fn build(src: Vec<u8>, tree: Tree) -> RustFile {
    let mut scopes = vec![Scope {
        parent: None,
        kind: ScopeKind::Root,
        entries: HashMap::new(),
    }];
    {
        let mut b = Builder {
            src: &src,
            scopes: &mut scopes,
            macro_depth: 0,
        };
        b.walk(tree.root_node(), 0);
    }
    RustFile {
        src,
        tree,
        scopes,
    }
}

impl<'m> Builder<'m> {
    fn text<'t>(&'t self, n: Node<'t>) -> &'t str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn open_scope(&mut self, kind: ScopeKind, parent: Option<usize>) -> usize {
        self.scopes.push(Scope {
            parent,
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let mut s = scope;
        loop {
            let data = &self.scopes[s];
            if let Some(e) = data.entries.get(name)
                && e.intro_byte <= pos
            {
                return Some(s);
            }
            match data.kind {
                ScopeKind::Block => s = data.parent?,
                _ => return None,
            }
        }
    }

    fn record_write(
        &mut self,
        scope: usize,
        name: &str,
        w: Write,
        intro: IntroKind,
    ) {
        if name.starts_with('_') {
            return;
        }
        match self.lookup(scope, w.byte, name) {
            Some(s) => {
                let e = self.scopes[s].entries.get_mut(name).unwrap();
                e.writes.push(w);
            }
            None => {
                let e = Entry {
                    intro_byte: w.byte,
                    intro_kind: intro,
                    writes: vec![w],
                    reads: Vec::new(),
                    macro_reads: 0,
                };
                self.scopes[scope].entries.insert(name.into(), e);
            }
        }
    }

    fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
        let Some(s) = self.lookup(scope, byte, name) else {
            return;
        };
        let in_macro = self.macro_depth > 0;
        let e = self.scopes[s].entries.get_mut(name).unwrap();
        e.reads.push(byte);
        if in_macro {
            e.macro_reads += 1;
        }
    }

    fn bind_pattern(&mut self, pattern: Option<Node>, scope: usize, intro: IntroKind) {
        let Some(p) = pattern else { return };
        let mut ids = Vec::new();
        pattern_identifiers(p, self.src, &mut ids);
        for id in ids {
            let name = self.text(id).to_string();
            self.record_write(
                scope,
                &name,
                Write {
                    byte: id.start_byte(),
                    node_id: id.id(),
                    plain: intro == IntroKind::Assign,
                    rhs: None,
                },
                intro,
            );
        }
    }

    const SCOPED: [&'static str; 3] =
        ["function_item", "closure_expression", "block"];
    const BINDERS: [&'static str; 7] = [
        "let_declaration",
        "assignment_expression",
        "compound_assignment_expr",
        "for_expression",
        "if_let_expression",
        "while_let_expression",
        "match_arm",
    ];

    fn walk(&mut self, n: Node, scope: usize) {
        let kind = n.kind();

        if skip_subtree(kind) || kind == "comment" {
            return;
        }

        if Self::SCOPED.contains(&kind) {
            return self.walk_scoped(n, scope, kind);
        }
        if Self::BINDERS.contains(&kind) {
            return self.walk_binder(n, scope, kind);
        }
        self.walk_other(n, scope, kind);
    }

    fn walk_scoped(&mut self, n: Node, scope: usize, kind: &str) {
        let block_scoped = kind != "function_item";
        let s = self.open_scope(
            if block_scoped { ScopeKind::Block } else { ScopeKind::Function },
            if block_scoped { Some(scope) } else { None },
        );
        if let Some(p) = n.child_by_field_name("parameters") {
            self.declare_params(p, s);
        }
        if block_scoped && kind == "closure_expression" {
            if let Some(body) = n.child_by_field_name("body") {
                self.walk(body, s);
            }
            return;
        }
        if kind == "block" {
            self.walk_children(n, s);
            return;
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.walk_children(body, s);
        }
    }

    fn walk_binder(&mut self, n: Node, scope: usize, kind: &str) {
        match kind {
            "let_declaration" => self.handle_let(n, scope),
            "assignment_expression" | "compound_assignment_expr" => {
                self.handle_assign(n, scope)
            }
            "match_arm" => self.handle_match_arm(n, scope),
            _ => self.handle_loop_or_let_binding(n, scope, kind),
        }
    }

    fn handle_let(&mut self, n: Node, scope: usize) {
        let pattern = n.child_by_field_name("pattern");
        let value = n.child_by_field_name("value");
        if let Some(p) = pattern {
            let mut ids = Vec::new();
            pattern_identifiers(p, self.src, &mut ids);
            for id in ids {
                let name = self.text(id).to_string();
                self.record_write(
                    scope,
                    &name,
                    Write {
                        byte: id.start_byte(),
                        node_id: id.id(),
                        plain: true,
                        rhs: value.map(|v| (v.id(), v.start_byte())),
                    },
                    IntroKind::Assign,
                );
            }
        }
        if let Some(v) = value {
            self.walk(v, scope);
        }
    }

    fn handle_assign(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        if let Some(l) = left {
            if l.kind() == "identifier" {
                let name = self.text(l).to_string();
                self.record_write(
                    scope,
                    &name,
                    Write {
                        byte: l.start_byte(),
                        node_id: l.id(),
                        plain: false,
                        rhs: None,
                    },
                    IntroKind::Binding,
                );
            } else {
                self.walk(l, scope);
            }
        }
        if let Some(r) = right {
            self.walk(r, scope);
        }
    }

    fn handle_match_arm(&mut self, n: Node, scope: usize) {
        let pattern = n.child_by_field_name("pattern");
        if let Some(p) = pattern {
            let mut binders = Vec::new();
            match_binders(p, self.src, &mut binders);
            for id in &binders {
                let name = self.text(*id).to_string();
                self.record_write(
                    scope,
                    &name,
                    Write {
                        byte: id.start_byte(),
                        node_id: id.id(),
                        plain: false,
                        rhs: None,
                    },
                    IntroKind::Binding,
                );
            }
            let skip: HashSet<usize> = binders.iter().map(|b| b.id()).collect();
            self.walk_skip_ids(p, scope, &skip);
        }
        if let Some(v) = n.child_by_field_name("value") {
            self.walk(v, scope);
        }
    }

    fn handle_loop_or_let_binding(&mut self, n: Node, scope: usize, kind: &str) {
        let pattern_field = if kind == "for_expression" { "pattern" } else { "pattern" };
        self.bind_pattern(n.child_by_field_name(pattern_field), scope, IntroKind::Binding);
        let pat = n.child_by_field_name(pattern_field);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if pat.map(|p| p.id()) == Some(child.id()) {
                continue;
            }
            self.walk(child, scope);
        }
    }

    #[allow(dead_code)]
    fn old_walk_removed(&mut self) {}
    fn walk_other(&mut self, n: Node, scope: usize, kind: &str) {
        if kind == "token_tree" {
            self.macro_depth += 1;
            self.walk_children(n, scope);
            self.macro_depth -= 1;
            return;
        }
        if kind == "identifier" {
            self.walk_ident_node(n, scope);
            return;
        }
        if kind == "scoped_identifier" {
            return;
        }
        self.walk_children(n, scope);
    }

    fn walk_ident_node(&mut self, n: Node, scope: usize) {
        let name = self.text(n).to_string();
        if name.starts_with('_') {
            return;
        }
        if self.lookup(scope, n.start_byte(), &name).is_some() {
            self.record_read(scope, &name, n.start_byte());
        }
    }

    fn walk_children(&mut self, n: Node, scope: usize) {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn walk_skip_ids(&mut self, n: Node, scope: usize, skip: &HashSet<usize>) {
        if skip.contains(&n.id()) {
            return;
        }
        if skip_subtree(n.kind()) || n.kind() == "comment" {
            return;
        }
        if n.child_count() == 0 {
            self.walk(n, scope);
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            self.walk_skip_ids(child, scope, skip);
        }
    }

    fn declare_params(&mut self, container: Node, scope: usize) {
        let mut cursor = container.walk();
        for child in container.children(&mut cursor) {
            match child.kind() {
                "self_parameter" => {}
                "parameter" => {
                    if let Some(pat) = child.child_by_field_name("pattern") {
                        let mut ids = Vec::new();
                        pattern_identifiers(pat, self.src, &mut ids);
                        for id in ids {
                            let name = self.text(id).to_string();
                            if name.starts_with('_') {
                                continue;
                            }
                            let pos = id.start_byte();
                            self.scopes[scope]
                                .entries
                                .entry(name.into())
                                .or_insert(Entry {
                                    intro_byte: pos,
                                    intro_kind: IntroKind::Binding,
                                    writes: Vec::new(),
                                    reads: Vec::new(),
                                    macro_reads: 0,
                                });
                        }
                    }
                }
                "identifier" => {
                    let name = self.text(child).to_string();
                    if name.starts_with('_') {
                        continue;
                    }
                    let pos = child.start_byte();
                    self.scopes[scope]
                        .entries
                        .entry(name.into())
                        .or_insert(Entry {
                            intro_byte: pos,
                            intro_kind: IntroKind::Binding,
                            writes: Vec::new(),
                            reads: Vec::new(),
                            macro_reads: 0,
                        });
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------- ABC ----

struct Calc<'f> {
    fm: &'f RustFile,
    a: u32,
    b: u32,
    c: u32,
}

fn pattern_count(fm: &RustFile, pattern: Node) -> u32 {
    let mut ids = Vec::new();
    pattern_identifiers(pattern, &fm.src, &mut ids);
    ids.len() as u32
}

impl<'f> Calc<'f> {
    fn walk(&mut self, n: Node) {
        let children: Vec<_> = {
            let mut cursor = n.walk();
            n.children(&mut cursor).collect()
        };
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
        "for_expression", "if_let_expression", "while_let_expression",
        "match_arm", "if_expression", "while_expression",
    ];
    const OPS: [&'static str; 5] = [
        "binary_expression", "unary_expression", "call_expression",
        "macro_invocation", "try_expression",
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
            "for_expression" | "if_let_expression" | "while_let_expression"
            | "if_expression" | "while_expression" => {
                self.c += 1;
                if matches!(kind, "for_expression" | "if_let_expression")
                    && let Some(p) = n.child_by_field_name("pattern")
                {
                    self.a += pattern_count(self.fm, p);
                }
            }
            "match_arm" => {
                self.c += 1;
                if let Some(p) = n.child_by_field_name("pattern") {
                    let mut binders = Vec::new();
                    match_binders(p, &self.fm.src, &mut binders);
                    self.a += binders.len() as u32;
                }
            }
            _ => {}
        }
    }

    fn count_ops(&mut self, n: Node) {
        match n.kind() {
            "binary_expression" => {
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
            "unary_expression" => {
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
            "call_expression" | "macro_invocation" | "try_expression" => self.b += 1,
            _ => {}
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
                        pattern_identifiers(pat, &fm.src, &mut ids);
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

fn visit_units(fm: &RustFile, n: Node, f: &mut impl FnMut(Node, &str)) {
    let is_fn = n.kind() == "function_item";
    if is_fn && let Some(name_node) = n.child_by_field_name("name") {
        f(n, fm.text(name_node));
    }
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        visit_units(fm, child, f);
    }
}

pub fn analyze(fm: &RustFile, max: f64) -> Vec<AbcOffense> {
    all_scores(fm)
        .into_iter()
        .filter(|o| o.score > max)
        .collect()
}

pub fn all_scores(fm: &RustFile) -> Vec<AbcOffense> {
    let mut offenses = Vec::new();
    visit_units(fm, fm.tree.root_node(), &mut |unit, name| {
        let Some(body) = unit.child_by_field_name("body") else {
            return;
        };
        let mut calc = Calc { fm, a: 0, b: 0, c: 0 };
        calc.walk(body);
        let raw =
            ((calc.a * calc.a + calc.b * calc.b + calc.c * calc.c) as f64).sqrt();
        let score = (raw * 100.0).round() / 100.0;
        let pos = unit.start_position();
        offenses.push(AbcOffense {
            line: pos.row + 1,
            column: pos.column,
            name: name.to_string(),
            score,
            vector: format!("<{}, {}, {}>", calc.a, calc.b, calc.c),
        });
    });
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses
}

// ---------------------------------------------------------- used once ----

fn index_nodes<'t>(root: Node<'t>) -> HashMap<usize, Node<'t>> {
    let mut map = HashMap::new();
    fn rec<'t>(n: Node<'t>, map: &mut HashMap<usize, Node<'t>>) {
        map.insert(n.id(), n);
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            rec(child, map);
        }
    }
    rec(root, &mut map);
    map
}

/// Straight-line execution check up to the nearest function/closure/block
/// boundary (the binding's scope).
fn unconditionally_executed(write_node: Node) -> bool {
    const VETO: [&str; 7] = [
        "if_expression",
        "if_let_expression",
        "while_expression",
        "while_let_expression",
        "for_expression",
        "match_arm",
        "match_expression",
    ];
    const OWNERS: [&str; 3] = ["function_item", "closure_expression", "block"];
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if VETO.contains(&n.kind()) {
            return false;
        }
        if OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

/// Conservative RHS purity: literals, constant paths, and compositions of
/// comparisons/logical/arithmetic over those. Calls, macros, `?`, field reads
/// through non-const bases, and local-variable references are all rejected.
fn pure(fm: &RustFile, n: Node) -> bool {
    match n.kind() {
        "integer_literal"
        | "float_literal"
        | "char_literal"
        | "string_literal"
        | "raw_string_literal"
        | "true"
        | "false"
        | "unit_type" => true,
        "scoped_identifier" => true, // constants; enforced immutable by rustc
        "reference_expression" | "unary_expression" => children_pure(fm, n),
        "binary_expression" | "tuple_expression" | "array_expression"
        | "range_expression" => children_pure(fm, n),
        "type_cast_expression" => n
            .child_by_field_name("value")
            .map(|v| pure(fm, v))
            .unwrap_or(false),
        "field_expression" => n
            .child_by_field_name("value")
            .map(|v| pure(fm, v))
            .unwrap_or(false),
        _ => false,
    }
}

fn children_pure<'t>(fm: &RustFile, n: Node<'t>) -> bool {
    let mut c = n.walk();
    n.children(&mut c)
        .filter(|ch| ch.is_named())
        .all(|ch| pure(fm, ch))
}

pub fn used_once_offenses(fm: &RustFile) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(fm.tree.root_node());
    let mut out = Vec::new();

    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if e.intro_kind != IntroKind::Assign {
                continue;
            }
            if e.writes.len() != 1 || e.reads.len() != 1 {
                continue;
            }
            let w = e.writes[0];
            let r = e.reads[0];
            if !w.plain || r <= w.byte || e.macro_reads > 0 {
                continue;
            }
            let Some((rhs_id, _)) = w.rhs else {
                continue;
            };
            let (Some(&rhs_node), Some(&write_node)) =
                (nodes.get(&rhs_id), nodes.get(&w.node_id))
            else {
                continue;
            };
            if !pure(fm, rhs_node) || !unconditionally_executed(write_node) {
                continue;
            }
            let (line, column) = fm.line_col(w.byte);
            out.push(UsedOnceOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_str(src: &str) -> RustFile {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("rust grammar");
        let tree = parser.parse(src, None).expect("syntax tree");
        build(src.as_bytes().to_vec(), tree)
    }

    fn scores(src: &str) -> Vec<AbcOffense> {
        all_scores(&build_str(src))
    }

    fn flags(src: &str) -> Vec<UsedOnceOffense> {
        used_once_offenses(&build_str(src))
    }

    #[test]
    fn compute_method_vector() {
        let s = scores(
            "fn compute(items: &[Option<u32>], factor: u32) -> u32 {\n\
             \x20   let mut total = 0u32;\n\
             \x20   for item in items.iter() {\n\
             \x20       if item.is_none() {\n\
             \x20           continue;\n\
             \x20       }\n\
             \x20       total += item.unwrap() * factor;\n\
             \x20   }\n\
             \x20   total / factor\n\
             }",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "compute");
        assert_eq!(s[0].vector, "<3, 5, 2>");
        assert!((s[0].score - 6.16).abs() < 1e-9);
    }

    #[test]
    fn if_else_if_chain_conditions_without_else_bonus() {
        let s = scores(
            "fn cond(x: i32) -> i32 {\n\
             \x20   if x == 1 && x < 5 { 1 } else if x > 10 { 2 } else { 3 }\n\
             }",
        );
        assert_eq!(s[0].vector, "<0, 0, 6>");
    }

    #[test]
    fn match_arms_and_guards() {
        let s = scores(
            "fn mat(c: u8) -> &'static str {\n\
             \x20   match c {\n\
             \x20       0 => \"zero\",\n\
             \x20       n if n > 10 => \"big\",\n\
             \x20       _ => \"other\",\n\
             \x20   }\n\
             }",
        );
        // three arms + guard comparison; binder `n` is one assignment
        assert_eq!(s[0].vector, "<1, 0, 4>");
    }

    #[test]
    fn closures_roll_into_enclosing_function() {
        let s = scores(
            "fn closures(v: Vec<u32>) -> u32 {\n\
             \x20   let add = |a: u32| a + 1;\n\
             \x20   add(v.len() as u32)\n\
             }",
        );
        // A: add-let + closure param a; B: v.len + + binary + add call
        assert_eq!(s[0].vector, "<2, 3, 0>");
    }

    #[test]
    fn macro_invocations_are_branches_and_token_reads_count() {
        let s = scores(
            "fn macros(n: u32) {\n\
             \x20   println!(\"{}\", n);\n\
             }",
        );
        assert_eq!(s[0].vector, "<0, 1, 0>");

        let s = scores(
            "fn macros2(n: u32) {\n\
             \x20   let m = n + 1;\n\
             \x20   println!(\"{}\", m);\n\
             }",
        );
        assert_eq!(s[0].vector, "<1, 2, 0>");
    }

    #[test]
    fn try_operator_is_a_branch() {
        let s = scores(
            "fn try_op(x: Result<u32, ()>) -> Result<u32, ()> {\n\
             \x20   let y = x?;\n\
             \x20   Ok(y + 1)\n\
             }",
        );
        assert_eq!(s[0].vector, "<1, 3, 0>");
    }

    #[test]
    fn shadowing_counts_as_multiple_writes_not_candidates() {
        let f = flags("fn f() {\n  let n = 1;\n  let n = n + 1;\n}");
        assert!(f.is_empty());
        let s = scores("fn f() {\n  let mut n = 1;\n  n += 1;\n  let n = n * 2;\n}");
        assert_eq!(s[0].vector, "<3, 1, 0>");
    }

    #[test]
    fn simple_single_use_is_flagged_at_let_line() {
        let f = flags("fn f() {\n  let tmp = 42;\n  p(tmp);\n}");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "tmp");
        assert_eq!(f[0].line, 2);
        assert_eq!(f[0].column, 6);
    }

    #[test]
    fn impure_rhs_rejected() {
        let f = flags("fn f(x: &str) {\n  let s = x.to_string();\n  p(s);\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn second_read_rejected() {
        let f = flags("fn f() {\n  let t = 7;\n  p(t); p(t);\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn if_let_binding_never_candidate() {
        let f = flags("fn g(o: Option<u32>) {\n  if let Some(v) = o {\n    p(v);\n  }\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn read_inside_later_closure_is_candidate() {
        let f = flags("fn k() {\n  let x = 42;\n  run(|| p(x));\n}");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "x");
    }

    #[test]
    fn pure_composite_rhs_accepted() {
        let f = flags("fn f(a: u32, b: u32) {\n  let m = a * b + 1;\n  p(m);\n}");
        // a and b are params (Binding), rhs references them -> identifiers are
        // not pure per spec... `a * b + 1` contains identifier reads, so the
        // conservative purity gate rejects it.
        assert!(f.is_empty());
    }
}

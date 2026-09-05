//! Scope-model collectors for the plain-C-family grammars (C, C++,
//! Objective-C). All three share tree-sitter's C core shapes -- reads are
//! `identifier`, declarations carry `init_declarator` (`@declarator`
//! identifier plus optional `@value`), assignments use the JS-shaped
//! `left`/`operator`/`right` fields, and member slots are distinct kinds
//! (`field_identifier`) needing no exclusion -- so one `Backend`
//! implementation serves every variant, discriminated only by its `Spec`.

use tree_sitter::Node;

use super::c_bind;
use crate::paths::Lang;
use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, ScopeKind, Write};

/// Skip macro *definitions* and includes only. Conditional bodies
/// (`preproc_ifdef` / `preproc_if`) are still walked so reads inside
/// `#if` / `#ifdef` branches count; misparsed `else if` is handled by
/// refusing to bind keywords in [`c_bind`].
const PREPROC: &[&str] = &[
    "preproc_include",
    "preproc_def",
    "preproc_function_def",
    "preproc_call",
];

/// Identifiers plus `type_identifier`: template args like
/// `StackBuffer<BufSize, char>` use the latter for the non-type value.
const C_READS: &[&str] = &["identifier", "type_identifier"];

const C_SPEC: Spec = Spec {
    skip_kinds: PREPROC,
    block_scoped: &["compound_statement", "lambda_expression"],
    function_kinds: &["function_definition", "method_definition"],
    read_kinds: C_READS,
    exclude_fields: &[],
};

/// Like [`C_SPEC`], but `function_definition` is handled in `custom` so
/// a misparsed `class EXPORT Name { ... }` body is not walked as locals.
const CPP_SPEC: Spec = Spec {
    skip_kinds: PREPROC,
    block_scoped: &["compound_statement", "lambda_expression"],
    function_kinds: &["method_definition"],
    read_kinds: C_READS,
    exclude_fields: &[],
};

const OBJC_SPEC: Spec = Spec {
    skip_kinds: &[
        "preproc_include",
        "preproc_def",
        "preproc_function_def",
        "preproc_call",
        // interface prototypes declare but define nothing; the
        // implementation section carries the bodies
        "class_interface",
        "protocol_declaration",
        "property_declaration",
        "method_declaration",
    ],
    block_scoped: &["compound_statement", "lambda_expression"],
    function_kinds: &["function_definition", "method_definition"],
    read_kinds: C_READS,
    exclude_fields: &[],
};

pub(super) fn collect(root: Node, src: &[u8], lang: Lang) -> Vec<crate::scope_model::Scope> {
    let mut c = Collector {
        src,
        model: Model::rooted(),
        lang,
    };
    dispatch(&mut c, root, 0);
    c.model.scopes
}

struct Collector<'a> {
    src: &'a [u8],
    model: Model,
    lang: Lang,
}

impl Backend for Collector<'_> {
    fn spec(&self) -> &'static Spec {
        match self.lang {
            Lang::Cpp => &CPP_SPEC,
            Lang::ObjC => &OBJC_SPEC,
            _ => &C_SPEC,
        }
    }

    fn model(&mut self) -> &mut Model {
        &mut self.model
    }

    fn text_of(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn custom(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "function_definition" => self.walk_function(n, scope),
            "declaration" | "field_declaration" => self.bind_declaration(n, scope),
            "assignment_expression" | "augmented_assignment_expression" => {
                self.bind_assignment(n, scope);
            }
            "update_expression" => self.bind_update(n, scope),
            // loop heads are protocol: do not bind `int i`, but still
            // collect reads in the initializer (`spans.rbegin()`).
            "for_statement" => self.walk_for_statement(n, scope),
            "for_range_statement" => self.walk_children_excluding_field(n, scope, "declarator"),
            _ => self.walk_children(n, scope),
        }
    }
}

/// True when tree-sitter took `class MACRO Name` as a function_definition
/// whose type is the class_specifier (export/visibility macros).
fn export_macro_class(n: Node) -> bool {
    matches!(
        n.child_by_field_name("type").map(|t| t.kind()),
        Some("class_specifier" | "struct_specifier" | "union_specifier")
    )
}

impl Collector<'_> {
    /// Open a function scope, unless this is a misparsed export-macro class.
    fn walk_function(&mut self, n: Node, scope: usize) {
        if export_macro_class(n) {
            return;
        }

        let s = self.model().open_scope(ScopeKind::Function, scope);
        self.walk_children(n, s);
    }

    /// i++ / ++i read-and-rewrite a visible local; non-local targets are
    /// operand reads only.
    fn bind_update(&mut self, n: Node, scope: usize) {
        let arg = n
            .child_by_field_name("argument")
            .or_else(|| n.named_child(0));
        if let Some(operand) = arg.filter(|o| o.kind() == "identifier") {
            self.rebind_local(operand, scope, false, None);
        } else if let Some(arg) = arg {
            dispatch(self, arg, scope);
        }
    }

    /// Bind each declarator of a `declaration`.
    fn bind_declaration(&mut self, n: Node, scope: usize) {
        // Statement macros / misparsed expressions (`emit`, `paths[I]=…`) — walk only.
        if c_bind::skip_declaration_bind(n, self.src) {
            self.walk_children(n, scope);
            return;
        }
        // C++ if-init `T* x = rhs` may put `@value` on the declaration itself.
        let decl_value = n.child_by_field_name("value");
        let in_block = n
            .parent()
            .is_some_and(|p| p.kind() == "compound_statement");
        let mut cursor = n.walk();
        for d in n.children(&mut cursor) {
            self.bind_decl_child(d, scope, decl_value, in_block);
        }
        self.dispatch_opt(decl_value, scope);
    }

    fn bind_decl_child(&mut self, d: Node, scope: usize, decl_value: Option<Node>, in_block: bool) {
        match d.kind() {
            "init_declarator" | "identifier" => self.bind_declarator(d, scope),
            // Local `QFile f(path)` — most-vexing-parse as function_declarator.
            "function_declarator" if in_block => self.bind_ctor_declarator(d, scope),
            // `T* x` / `T& x` (value may sit on the declaration in if-init)
            k if k.ends_with("_declarator") && k != "function_declarator" => {
                self.try_bind_local(d, scope, decl_value);
            }
            _ => {}
        }
    }

    /// `Type name(args)` inside a block: bind `name`, walk param slots as
    /// expression reads (`path` in `QFile f(path)` is a `type_identifier`).
    fn bind_ctor_declarator(&mut self, fd: Node, scope: usize) {
        let rhs = fd.child_by_field_name("parameters");
        self.try_bind_local(fd, scope, rhs);
        if let Some(params) = rhs {
            self.walk_ctor_args(params, scope);
        }
    }

    fn walk_ctor_args(&mut self, params: Node, scope: usize) {
        let mut cursor = params.walk();
        for p in params
            .children(&mut cursor)
            .filter(|ch| ch.kind() == "parameter_declaration")
        {
            if let Some(ty) = p.child_by_field_name("type") {
                if p.child_by_field_name("declarator").is_none() {
                    dispatch(self, ty, scope);
                    continue;
                }
            }
            self.walk_children(p, scope);
        }
    }

    /// Bind one declarator: bare `int x;` or `init_declarator` (possibly
    /// under pointer/array wrappers). Skip keywords/macros and RAII guards.
    fn bind_declarator(&mut self, d: Node, scope: usize) {
        let rhs = init_rhs(d);
        self.try_bind_local(d, scope, rhs);
        self.dispatch_opt(rhs, scope);
    }

    fn try_bind_local(&mut self, d: Node, scope: usize, rhs: Option<Node>) {
        let Some(name) = declarator_ident(d) else {
            return;
        };
        if !c_bind::should_bind(d, self.text_of(name), rhs, self.src) {
            return;
        }
        self.bind_var(
            name,
            scope,
            Write::assign(name.start_byte(), name.id(), rhs.map(|v| v.id())),
            IntroKind::Assign,
        );
        if condition_introduces(d) {
            let n = self.text_of(name).to_string();
            self.model().record_read(scope, &n, name.start_byte());
        }
    }

    fn dispatch_opt(&mut self, n: Option<Node>, scope: usize) {
        if let Some(n) = n {
            dispatch(self, n, scope);
        }
    }

    /// `for` head: skip binding init declarators, still walk their RHS.
    fn walk_for_statement(&mut self, n: Node, scope: usize) {
        if let Some(init) = n.child_by_field_name("initializer") {
            self.walk_for_init_reads(init, scope);
        }
        self.walk_children_excluding_field(n, scope, "initializer");
    }

    fn walk_for_init_reads(&mut self, n: Node, scope: usize) {
        match n.kind() {
            "init_declarator" => self.dispatch_opt(init_rhs(n), scope),
            "assignment_expression" | "augmented_assignment_expression" => {
                if let Some(right) = n.child_by_field_name("right") {
                    dispatch(self, right, scope);
                }
            }
            k if self.spec().read_kinds.contains(&k) => {
                dispatch(self, n, scope);
            }
            _ => {
                let mut c = n.walk();
                for ch in n.children(&mut c) {
                    self.walk_for_init_reads(ch, scope);
                }
            }
        }
    }

    /// Plain `=` rebinds a visible local (one candidate write); compound
    /// operators rewrite-and-read. C has globals -- targets no visible
    /// binding introduced are file-scope objects whose operands are reads
    /// only.
    fn bind_assignment(&mut self, n: Node, scope: usize) {
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        let plain = n
            .child_by_field_name("operator")
            .map_or(false, |o| self.text_of(o) == "=");
        if let Some(left) = left {
            if left.kind() == "identifier" {
                self.rebind_local(left, scope, plain, right.map(|r| r.id()));
            } else {
                // pointer derefs, field writes, array slots: operands only
                self.walk_children(left, scope);
            }
        }
        if let Some(right) = right {
            dispatch(self, right, scope);
        }
    }
}

fn declarator_ident(d: Node) -> Option<Node> {
    if d.kind() == "identifier" {
        Some(d)
    } else {
        d.child_by_field_name("declarator")
            .and_then(resolve_declarator_name)
    }
}

fn init_rhs(d: Node) -> Option<Node> {
    (d.kind() == "init_declarator")
        .then(|| d.child_by_field_name("value"))
        .flatten()
}

/// Strip declarator wrappers (`pointer_declarator`, `array_declarator`,
/// `function_declarator`, ...) until the named identifier underneath.
/// Prefer the `@declarator` field so a `* const ptr` qualifier is not
/// mistaken for the name (named_child(0) would be the `const`).
fn resolve_declarator_name<'tree>(mut decl: Node<'tree>) -> Option<Node<'tree>> {
    while decl.kind().ends_with("_declarator") && decl.kind() != "init_declarator" {
        decl = decl
            .child_by_field_name("declarator")
            .or_else(|| decl.named_child(0))?;
    }
    (decl.kind() == "identifier").then_some(decl)
}

/// True when this declarator is introduced as an `if`/`while`/`switch`
/// condition (C++ init-statement or declaration-as-condition).
fn condition_introduces(d: Node) -> bool {
    let Some(decl) = d.parent().filter(|p| p.kind() == "declaration") else {
        return false;
    };
    match decl.parent().map(|p| p.kind()) {
        Some("condition_clause") => true,
        Some("init_statement") => decl
            .parent()
            .and_then(|p| p.parent())
            .is_some_and(|g| g.kind() == "condition_clause"),
        _ => false,
    }
}

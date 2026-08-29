//! Scope-model collectors for the plain-C-family grammars (C, C++,
//! Objective-C). All three share tree-sitter's C core shapes -- reads are
//! `identifier`, declarations carry `init_declarator` (`@declarator`
//! identifier plus optional `@value`), assignments use the JS-shaped
//! `left`/`operator`/`right` fields, and member slots are distinct kinds
//! (`field_identifier`) needing no exclusion -- so one `Backend`
//! implementation serves every variant, discriminated only by its `Spec`.

use tree_sitter::Node;

use crate::paths::Lang;
use crate::scope_model::walk::{Backend, Spec, dispatch};
use crate::scope_model::{IntroKind, Model, ScopeKind, Write};

const PREPROC: &[&str] = &[
    "preproc_include",
    "preproc_def",
    "preproc_function_def",
    "preproc_call",
    "preproc_ifdef",
    "preproc_if",
];

const C_SPEC: Spec = Spec {
    skip_kinds: PREPROC,
    block_scoped: &["compound_statement", "lambda_expression"],
    function_kinds: &["function_definition", "method_definition"],
    read_kinds: &["identifier"],
    exclude_fields: &[],
};

/// Like [`C_SPEC`], but `function_definition` is handled in `custom` so
/// a misparsed `class EXPORT Name { ... }` body is not walked as locals.
const CPP_SPEC: Spec = Spec {
    skip_kinds: PREPROC,
    block_scoped: &["compound_statement", "lambda_expression"],
    function_kinds: &["method_definition"],
    read_kinds: &["identifier"],
    exclude_fields: &[],
};

const OBJC_SPEC: Spec = Spec {
    skip_kinds: &[
        "preproc_include",
        "preproc_def",
        "preproc_function_def",
        "preproc_call",
        "preproc_ifdef",
        "preproc_if",
        // interface prototypes declare but define nothing; the
        // implementation section carries the bodies
        "class_interface",
        "protocol_declaration",
        "property_declaration",
        "method_declaration",
    ],
    block_scoped: &["compound_statement", "lambda_expression"],
    function_kinds: &["function_definition", "method_definition"],
    read_kinds: &["identifier"],
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
            // loop heads are protocol: initializer/control variables are
            // never tracked (an `int i` written once and read once would
            // otherwise surface a bogus inlining suggestion)
            "for_statement" => self.walk_children_excluding_field(n, scope, "initializer"),
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
        let mut cursor = n.walk();
        for d in n
            .children(&mut cursor)
            .filter(|ch| ch.kind() == "init_declarator" || ch.kind() == "identifier")
        {
            self.bind_declarator(d, scope);
        }
    }

    /// Bind one declarator position: either the identifier itself (bare
    /// `int x;`) or an init_declarator whose @declarator may sit under
    /// pointer/array/function wrappers. An initializer links as the
    /// inlinable RHS; a bare definition is a valueless write.
    fn bind_declarator(&mut self, d: Node, scope: usize) {
        if d.kind() == "identifier" {
            let w = Write::assign(d.start_byte(), d.id(), None);
            self.bind_var(d, scope, w, IntroKind::Assign);
            return;
        }
        let Some(name) = d
            .child_by_field_name("declarator")
            .and_then(resolve_declarator_name)
        else {
            return;
        };
        let rhs = d.child_by_field_name("value");
        let w = Write::assign(name.start_byte(), name.id(), rhs.map(|v| v.id()));
        self.bind_var(name, scope, w, IntroKind::Assign);
        // initializer subtrees may hold nested declarations
        if let Some(value) = rhs {
            dispatch(self, value, scope);
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

/// Strip declarator wrappers (`pointer_declarator`, `array_declarator`,
/// `function_declarator`, ...) until the named identifier underneath.
fn resolve_declarator_name<'tree>(mut decl: Node<'tree>) -> Option<Node<'tree>> {
    while decl.kind().ends_with("_declarator") && decl.kind() != "init_declarator" {
        decl = decl.named_child(0)?;
    }
    (decl.kind() == "identifier").then_some(decl)
}

//! The [`Backend`] contract: the three trivial accessors a language
//! collector implements plus the shared default bindings every backend
//! inherits -- plain binds, declarator-with-initializer binds, child
//! walking, and local rebinding.

use tree_sitter::Node;

use super::walk::{Spec, dispatch};
use super::{IntroKind, Model, Write};

/// A backend collector driving the shared [`Model`].
pub trait Backend {
    fn spec(&self) -> &'static Spec;
    fn model(&mut self) -> &mut Model;
    fn text_of(&self, n: Node) -> &str;
    /// Language-specific arms. Everything the [`Spec`] covers has been
    /// consumed before this runs; fall back to walking children for the
    /// remainder.
    fn custom(&mut self, n: Node, scope: usize);

    /// Bind `name_node` (text used verbatim, underscore-filtered by the
    /// model) with the given write.
    fn bind_var(&mut self, name_node: Node, scope: usize, w: Write, intro: IntroKind) {
        let name = self.text_of(name_node).to_string();
        self.model().bind(scope, &name, w, intro);
    }
    /// Bind a `variable_declarator`'s `@name`, linking its initializer
    /// as the inlinable RHS when the grammar exposes one.
    fn bind_declarator_with_rhs_field(&mut self, n: Node, scope: usize)
    where
        Self: Sized,
    {
        if let Some(name) = n.child_by_field_name("name")
            && name.kind() == "identifier"
        {
            // grammars differ on exposing an initializer field; when
            // absent, it is the first named child after the `=` token
            let rhs = n
                .child_by_field_name("value")
                .map(|v| v.id())
                .or_else(|| rhs_after_eq(self as &dyn Backend, n).map(|v| v.id()));
            let w = Write::assign(name.start_byte(), name.id(), rhs);
            self.bind_var(name, scope, w, IntroKind::Assign);
        }
    }

    /// Dispatch every child of `n` (in order), the shared fall-through for
    /// every backend's `_ =>` custom arm and inline-statement walking.
    fn walk_children(&mut self, n: Node, scope: usize)
    where
        Self: Sized,
    {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            dispatch(self, child, scope);
        }
    }

    /// Like [`Backend::walk_children`] but skips the child carried by `field`
    /// (used for protocol heads and member-access slots that are not reads).
    fn walk_children_excluding_field(&mut self, n: Node, scope: usize, field: &str)
    where
        Self: Sized,
    {
        let excluded = n.child_by_field_name(field).map(|c| c.id());
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if Some(child.id()) != excluded {
                dispatch(self, child, scope);
            }
        }
    }

    /// Bind/rebind a visible local named `left`: plain `=` is an `Assign`
    /// write (linking `rhs` as the inlineable expression); compound
    /// operators are a rewrite that also reads the previous value. Returns
    /// whether a visible local was rebound -- callers treat a false result
    /// as "no local binding here" and walk the operand(s) as reads only.
    fn rebind_local(&mut self, left: Node, scope: usize, plain: bool, rhs: Option<usize>) -> bool {
        let name = self.text_of(left).to_string();
        if self
            .model()
            .lookup(scope, left.start_byte(), &name)
            .is_none()
        {
            return false;
        }
        let (w, intro) = write_for_rebind(left, plain, rhs);
        self.model().bind(scope, &name, w, intro);
        if !plain {
            self.model().record_read(scope, &name, left.end_byte());
        }
        true
    }
}

/// Build the `Write`/`IntroKind` pair for a rebinding assignment: plain `=`
/// links `rhs` as the inlinable expression and qualifies as an inline
/// candidate; compound operators rewrite-and-read (a `Binding`, never a
/// candidate). Factored out of [`Backend::rebind_local`] to keep the
/// single-write path below the ABC threshold and free of call-chain
/// duplication.
fn write_for_rebind(left: Node, plain: bool, rhs: Option<usize>) -> (Write, IntroKind) {
    if plain {
        (
            Write::assign(left.start_byte(), left.id(), rhs),
            IntroKind::Assign,
        )
    } else {
        (
            Write::rewrite(left.start_byte(), left.id()),
            IntroKind::Binding,
        )
    }
}

/// First named child after the top-level `=` token, for grammars that do
/// not expose an initializer field (e.g. C#'s `variable_declarator`).
fn rhs_after_eq<'a>(b: &'a dyn Backend, n: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = n.walk();
    let mut after_eq = false;
    n.children(&mut cursor).find(|ch| {
        if !ch.is_named() && b.text_of(*ch) == "=" {
            after_eq = true;
            false
        } else {
            after_eq && ch.is_named()
        }
    })
}

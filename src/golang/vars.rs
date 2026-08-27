//! Variable write/read model powering UsedOnce/NeverUsed for Go.
//!
//! Scope rules mirror the Rust backend: reads resolve through Block
//! scopes -- which here include function literals, explicit blocks and
//! the implicit scopes of if/for/switch statements -- but stop at
//! Function boundaries. A read before the binding's introduction never
//! counts; Go rejects use-before-declaration at compile time anyway.

mod bindings;
mod collector;
mod purity;
mod report;

use std::collections::HashMap;

pub(super) use collector::collect;
pub use report::{never_used_offenses, used_once_offenses};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntroKind {
    /// `:=`, `=` first introduction, var spec -- inline candidate
    Assign,
    /// compound assignment or inc/dec -- never a candidate
    Binding,
}

#[derive(Clone, Copy, Debug)]
struct Write {
    byte: usize,
    node_id: usize,
    plain: bool,
    rhs: Option<usize>,
}

struct Entry {
    intro_byte: usize,
    intro_kind: IntroKind,
    writes: Vec<Write>,
    reads: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Root,
    Function,
    Block,
}

pub(super) struct Scope {
    parent: Option<usize>,
    kind: ScopeKind,
    entries: HashMap<Box<str>, Entry>,
}

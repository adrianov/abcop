//! Shared variable-scope model behind UsedOnce/NeverUsed: the write/
//! read bookkeeping every non-C backend needs, plus the candidate
//! evaluation itself. Language backends own only what genuinely differs
//! -- their collector walk (which nodes bind or read) and their RHS
//! purity whitelist, supplied here as [`Semantics`].
//!
//! Scope resolution contract: a name resolves to the nearest enclosing
//! scope that already introduced it at the read position; resolution
//! escapes through [`ScopeKind::Block`] scopes (closures, nested
//! blocks) and stops at any other kind (functions, methods).

use std::collections::HashMap;

use tree_sitter::Node;

use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntroKind {
    /// first introduction is an ordinary binding -- inline candidate
    Assign,
    /// introduced by a rewrite (compound assignment, catch/resource
    /// binding) -- never a candidate
    Binding,
}

#[derive(Clone, Copy, Debug)]
pub struct Write {
    pub byte: usize,
    pub node_id: usize,
    pub plain: bool,
    pub rhs: Option<usize>,
}

impl Write {
    /// A rebindable write whose value can be linked to an expression.
    pub fn assign(byte: usize, node_id: usize, rhs: Option<usize>) -> Write {
        Write {
            byte,
            node_id,
            plain: true,
            rhs,
        }
    }

    /// A rewriting touch (`+=`, catch/resource head): reads the previous
    /// value and can never be inlined away.
    pub fn rewrite(byte: usize, node_id: usize) -> Write {
        Write {
            byte,
            node_id,
            plain: false,
            rhs: None,
        }
    }
}

#[derive(Debug)]
pub struct Entry {
    pub intro_byte: usize,
    pub intro_kind: IntroKind,
    pub writes: Vec<Write>,
    pub reads: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Root,
    Function,
    Block,
}

#[derive(Debug)]
pub struct Scope {
    pub parent: Option<usize>,
    pub kind: ScopeKind,
    pub entries: HashMap<Box<str>, Entry>,
}

/// Bookkeeping over a scope tree: introduction-aware lookups and
/// write/read recording. Collectors drive it while walking.
#[derive(Debug, Default)]
pub struct Model {
    pub scopes: Vec<Scope>,
}

impl Model {
    pub fn rooted() -> Model {
        Model {
            scopes: vec![Scope {
                parent: None,
                kind: ScopeKind::Root,
                entries: HashMap::new(),
            }],
        }
    }

    pub fn open_scope(&mut self, kind: ScopeKind, parent: usize) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            kind,
            entries: HashMap::new(),
        });
        self.scopes.len() - 1
    }

    /// Name exists in this scope (a same-scope declaration shadows outer
    /// scopes regardless of position); the use counts only when the
    /// binding was already introduced at `pos`.
    pub fn lookup(&self, scope: usize, pos: usize, name: &str) -> Option<usize> {
        let data = &self.scopes[scope];
        if let Some(e) = data.entries.get(name) {
            return if e.intro_byte <= pos { Some(scope) } else { None };
        }
        match data.kind {
            ScopeKind::Block => self.lookup(data.parent?, pos, name),
            _ => None,
        }
    }

    pub fn bind(&mut self, scope: usize, name: &str, w: Write, intro: IntroKind) {
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

    pub fn record_read(&mut self, scope: usize, name: &str, byte: usize) {
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
}

/// The parts of candidate evaluation that differ per language.
pub struct Semantics {
    /// Conservative RHS purity: may this expression be inlined without
    /// changing behavior?
    pub pure: fn(Node) -> bool,
    /// Ancestors that mark a write as conditional (kills inlining).
    pub veto: &'static [&'static str],
    /// Ancestors that end the straight-line check (unit boundaries).
    pub owners: &'static [&'static str],
}

/// Inline candidates: one plain introduction, one later resolved read,
/// pure RHS, written on a straight-line path.
pub fn used_once_offenses(
    root: Node,
    line_col: &dyn Fn(usize) -> (usize, usize),
    scopes: &[Scope],
    sem: &Semantics,
) -> Vec<UsedOnceOffense> {
    let nodes = index_nodes(root);
    let mut out = Vec::new();
    for scope in scopes {
        for (name, e) in &scope.entries {
            let Some(w) = candidate(e) else {
                continue;
            };
            let Some(rhs_id) = w.rhs else { continue };
            let (Some(rhs), Some(write_node)) = (nodes.get(&rhs_id), nodes.get(&w.node_id))
            else {
                continue;
            };
            if !(sem.pure)(*rhs) || !straight_line(*write_node, sem) {
                continue;
            }
            let (line, column) = line_col(w.byte);
            out.push(UsedOnceOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    finish(out)
}

fn candidate(e: &Entry) -> Option<&Write> {
    if e.intro_kind != IntroKind::Assign || e.writes.len() != 1 || e.reads.len() != 1 {
        return None;
    }
    let w = &e.writes[0];
    (w.plain && e.reads[0] > w.byte).then_some(w)
}

fn straight_line(write_node: Node, sem: &Semantics) -> bool {
    let mut cur = Some(write_node);
    while let Some(n) = cur {
        if sem.veto.contains(&n.kind()) {
            return false;
        }
        if sem.owners.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}

/// Dead writes: bindings with at least one write and no resolved read,
/// reported once at the first write.
pub fn never_used_offenses(
    line_col: &dyn Fn(usize) -> (usize, usize),
    scopes: &[Scope],
) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();
    for scope in scopes {
        for (name, e) in &scope.entries {
            if !e.reads.is_empty() || e.writes.is_empty() {
                continue;
            }
            let first = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
            let (line, column) = line_col(first);
            out.push(NeverUsedOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    finish(out)
}

fn finish<T>(mut out: Vec<T>) -> Vec<T>
where
    T: HasPos + PartialEq,
{
    out.sort_by_key(|o| (o.line(), o.column()));
    out.dedup_by(|a, b| a.line() == b.line() && a.column() == b.column() && a.name() == b.name());
    out
}

trait HasPos {
    fn line(&self) -> usize;
    fn column(&self) -> usize;
    fn name(&self) -> &str;
}

macro_rules! impl_has_pos {
    ($t:ty) => {
        impl HasPos for $t {
            fn line(&self) -> usize {
                self.line
            }
            fn column(&self) -> usize {
                self.column
            }
            fn name(&self) -> &str {
                &self.name
            }
        }
    };
}

impl_has_pos!(UsedOnceOffense);
impl_has_pos!(NeverUsedOffense);

/// First child of the given kind -- the small lookup every collector
/// ends up needing for protocol heads and bindings.
pub(crate) fn child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).find(|ch| ch.kind() == kind)
}

fn index_nodes<'t>(root: Node<'t>) -> HashMap<usize, Node<'t>> {
    let mut map = HashMap::new();
    rec(root, &mut map);
    map
}

fn rec<'t>(n: Node<'t>, map: &mut HashMap<usize, Node<'t>>) {
    map.insert(n.id(), n);
    let mut cursor = n.walk();
    for child in n.children(&mut cursor) {
        rec(child, map);
    }
}

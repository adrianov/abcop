//! Scan-scope resolution: decides which repository state an invocation
//! reviews. A bare invocation picks the narrowest scope still reviewing
//! real work: uncommitted work against HEAD when there is any, the MR
//! scope (branch changes vs base) when the tree is clean.
//! `--uncommitted` pins the working tree only; `--mr` pins the branch
//! union. Outside a repository, or with no detectable base, the bare
//! default falls back to a full-tree scan rather than failing.
//!
//! Diff selection itself lives in [`crate::git_changes`]; base-ref choice
//! lives in [`crate::mr_scope`].

use std::process::ExitCode;

use crate::git_changes::{Changeset, Lines};

/// Resolve which git-scope applies to this invocation. `Ok(None)` means
/// no repository scoping: analyse explicit targets or the whole tree.
/// `uncommitted` selects working-tree work vs HEAD only, and fails
/// loudly outside a repository -- an explicitly narrowed scope must not
/// silently widen. The bare default is smart: uncommitted work when
/// present, the MR scope otherwise.
pub(crate) fn resolve(
    mr: bool,
    uncommitted: bool,
    explicit_paths: bool,
    full: bool,
    everything: bool,
) -> Result<Option<Changeset>, String> {
    if !(mr || uncommitted || (!explicit_paths && !full && !everything)) {
        return Ok(None);
    }
    if uncommitted {
        return Changeset::load("HEAD").map(Some);
    }
    let head = Changeset::load("HEAD");
    if mr {
        return branch_scope(head);
    }
    smart_default(head)
}

/// Bare-invocation default: the narrowest scope still reviewing real
/// work. Uncommitted changes win -- they are what is being edited right
/// now; a clean tree scans the branch's MR scope instead. The choice is
/// announced: a silently narrowed scope looks identical to a requested
/// one, which is exactly what misleads.
fn smart_default(head: Result<Changeset, String>) -> Result<Option<Changeset>, String> {
    match head {
        Ok(cs) if !cs.files.is_empty() => {
            eprintln!(
                "note: uncommitted changes detected; scanning uncommitted work only \
                 (--mr adds the branch diff vs its base)"
            );
            Ok(Some(cs))
        }
        head => branch_scope(head),
    }
}

/// MR union: the branch diff vs its base plus uncommitted HEAD work,
/// falling back per [`head_fallback`] when the base cannot be resolved.
fn branch_scope(head: Result<Changeset, String>) -> Result<Option<Changeset>, String> {
    match load_mr_scope() {
        Ok(mr_cs) => Ok(Some(union_with_head(mr_cs, head))),
        Err(e) => head_fallback(head, &e),
    }
}

/// When the base cannot be resolved: fall back to uncommitted HEAD work
/// alone, or to the whole tree when even that is empty or unavailable.
/// Either way the reason is printed -- a silently narrowed scope looks
/// identical to the requested one, which is exactly what misleads.
fn head_fallback(
    head: Result<Changeset, String>,
    reason: &str,
) -> Result<Option<Changeset>, String> {
    match head {
        Ok(h) if !h.files.is_empty() => {
            eprintln!("note: no MR scope ({reason}); scanning uncommitted HEAD work only");
            Ok(Some(h))
        }
        _ => {
            eprintln!("note: no MR scope ({reason}); scanning the full tree");
            Ok(None)
        }
    }
}

/// Union of the MR base scope with uncommitted HEAD work over the same
/// repository: a file present in either counts, and per-line ranges
/// widen (`All` dominates).
fn union_with_head(base: Changeset, extra: Result<Changeset, String>) -> Changeset {
    let Ok(extra) = extra else { return base };
    use std::collections::btree_map::Entry;
    let mut base = base;
    for (path, lines) in extra.files {
        match base.files.entry(path) {
            Entry::Vacant(v) => {
                v.insert(lines);
            }
            Entry::Occupied(mut o) => {
                let merged = match (o.get(), lines) {
                    (Lines::All, _) | (_, Lines::All) => Lines::All,
                    (Lines::Ranges(a), Lines::Ranges(b)) => {
                        let mut u = a.clone();
                        u.extend(b);
                        Lines::Ranges(u)
                    }
                };
                o.insert(merged);
            }
        }
    }
    base
}

fn load_mr_scope() -> Result<Changeset, String> {
    let (base, label) = crate::mr_scope::mr_base()?;
    eprintln!("--mr scope: {label} (base {base})");
    Changeset::load(&base)
}

/// Scope resolution failure: report it and use the scope-error exit code.
pub(crate) fn error(e: String) -> ExitCode {
    eprintln!("{e}");
    ExitCode::from(2)
}

/// Hint when a scoped run matched nothing: likely a stale scope.
pub(crate) fn note_if_empty(changeset: Option<&Changeset>, files: &[std::path::PathBuf]) {
    if changeset.is_some() && files.is_empty() {
        eprintln!("note: nothing changed in scope; try --full to scan the whole tree");
    }
}

//! Scan-scope resolution: decides which repository state an invocation
//! reviews. Bare invocations default to the MR scope -- uncommitted work
//! against HEAD plus everything the branch already changed vs its base,
//! the union CI would gate on. Outside a repository, or with no
//! detectable base, they fall back to a full-tree scan rather than
//! failing.
//!
//! Diff selection itself lives in [`crate::git_changes`]; base-ref choice
//! lives in [`crate::mr_scope`].

use std::process::ExitCode;

use crate::git_changes::{Changeset, Lines};

/// Resolve which git-scope applies to this invocation. `Ok(None)` means
/// no repository scoping: analyse explicit targets or the whole tree.
pub(crate) fn resolve(
    mr: bool,
    explicit_paths: bool,
    full: bool,
    everything: bool,
) -> Result<Option<Changeset>, String> {
    if !(mr || (!explicit_paths && !full && !everything)) {
        return Ok(None);
    }
    let head = Changeset::load("HEAD");
    match load_mr_scope() {
        Ok(mr_cs) => Ok(Some(union_with_head(mr_cs, head))),
        Err(e) => head_fallback(head, &e),
    }
}

/// When the base cannot be resolved: fall back to uncommitted HEAD work
/// alone, or to the whole tree when even that is empty or unavailable.
fn head_fallback(head: Result<Changeset, String>, reason: &str) -> Result<Option<Changeset>, String> {
    match head {
        Ok(h) if !h.files.is_empty() => Ok(Some(h)),
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

//! Repository-state queries over the built-in `git2` binding: which
//! branch is checked out, what counts as the default branch, commit-ish
//! resolution and ancestry tests. No external `git` process is ever
//! spawned at runtime; every lookup opens a repository handle directly.

use std::path::Path;

use git2::Repository;

const DEFAULT_BRANCHES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

/// Opens the enclosing repository without spawning git: discovery walks
/// parents, so a call from a subdirectory behaves like running git
/// inside it.
pub(crate) fn open_repo(dir: Option<&Path>) -> Result<Repository, String> {
    match dir {
        Some(d) => Repository::discover(d),
        None => Repository::discover("."),
    }
    .map_err(|e| format!("no usable git repository: {e}"))
}

pub(crate) fn rev_exists_in(dir: Option<&Path>, rev: &str) -> bool {
    open_repo(dir)
        .and_then(|r| {
            r.revparse_single(rev)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .is_ok()
}

pub(crate) fn current_branch_in(dir: Option<&Path>) -> Option<String> {
    let repo = open_repo(dir).ok()?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        // detached HEAD: nothing branch-like to report
        return None;
    }
    head.shorthand().map(str::to_string)
}

pub(crate) fn detect_default_branch_in(dir: Option<&Path>) -> Option<&'static str> {
    DEFAULT_BRANCHES
        .iter()
        .copied()
        .find(|c| rev_exists_in(dir, c))
}

pub(crate) fn is_ancestor_in(dir: Option<&Path>, a: &str, b: &str) -> bool {
    let resolved = || -> Result<(git2::Oid, git2::Oid), String> {
        let repo = open_repo(dir)?;
        let oid_a = commit_oid(&repo, a)?;
        let oid_b = commit_oid(&repo, b)?;
        Ok((oid_a, oid_b))
    };
    matches!(resolved(), Ok((a, b)) if a == b || repo_descendant(dir, b, a))
}

fn repo_descendant(dir: Option<&Path>, descendant: git2::Oid, ancestor: git2::Oid) -> bool {
    open_repo(dir)
        .and_then(|r| {
            r.graph_descendant_of(descendant, ancestor)
                .map_err(|e| e.to_string())
        })
        .unwrap_or(false)
}

/// Resolves any commit-ish to its object id.
pub(crate) fn commit_oid(repo: &Repository, rev: &str) -> Result<git2::Oid, String> {
    repo.revparse_single(rev)
        .map_err(|e| format!("cannot resolve {rev}: {e}"))?
        .peel_to_commit()
        .map(|c| c.id())
        .map_err(|e| format!("{rev} is not a commit: {e}"))
}

/// HEAD detached or sitting on the default branch under either spelling.
pub(crate) fn on_default_branch(branch: &str, default_branch: &str) -> bool {
    if branch == "HEAD" {
        return true;
    }
    let bare = default_branch
        .strip_prefix("origin/")
        .unwrap_or(default_branch);
    branch == bare || branch == default_branch
}

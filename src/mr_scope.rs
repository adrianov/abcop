//! Merge-request base-ref resolution: decides what an MR-scoped scan
//! diffs against.
//!
//! Preference order:
//! 1. The parent branch's fork point -- the newest commit shared by the
//!    current branch and any sibling -- so chained feature branches diff
//!    against their immediate parent instead of master.
//! 2. On a topic branch with no sibling: merge-base with the default
//!    branch.
//! 3. On the default branch itself (or detached): the last 36 hours.
//!
//! Every lookup takes an optional directory so tests can exercise
//! repository topologies without mutating the process-wide working
//! directory.

use std::path::Path;

use crate::fork_point::fork_point_in;
use crate::repo_state::{
    commit_oid, current_branch_in, detect_default_branch_in, on_default_branch, open_repo,
};

/// Default-branch scope: everything committed in the last 36 hours.
///
/// Both queries run against HEAD, never against a remote-tracking ref:
/// commits sitting on a fetched tip ahead of the checkout are other
/// people's work, and anchoring on their side of history floods the
/// diff with out-of-window drift. Reflog dates (`@{36.hours.ago}`)
/// share the same flaw -- with a sparse reflog git clamps them to the
/// oldest stored entry, silently widening the window to "since the
/// last fetch" -- so commit dates are used exclusively here.
fn aged_default_base_in(dir: Option<&Path>) -> Result<(String, String), String> {
    let repo = open_repo(dir)?;
    let (recent, edge) = walk_window(&repo, now_epoch() - WINDOW_SECS)?;
    if recent == 0 {
        return Err("no commits in the last 36 hours".to_string());
    }

    Ok((
        edge.unwrap_or_else(|| "HEAD".to_string()),
        "last 36 hours".to_string(),
    ))
}

/// One time-sorted walk over HEAD ancestry: counts in-window commits
/// and stops at the newest commit outside the window.
fn walk_window(repo: &git2::Repository, cutoff: i64) -> Result<(usize, Option<String>), String> {
    let walk = head_walk(repo)?;
    let mut recent = 0;
    for oid in walk {
        let oid = oid.map_err(|e| e.to_string())?;
        let ts = commit_time(repo, oid)?;
        if ts < cutoff {
            // Diffing from it reaches exactly the window's content plus
            // working-tree state.
            return Ok((recent, Some(oid.to_string())));
        }
        recent += 1;
    }
    Ok((recent, None))
}

/// Committer epoch of a walked oid.
fn commit_time(repo: &git2::Repository, oid: git2::Oid) -> Result<i64, String> {
    Ok(repo
        .find_commit(oid)
        .map_err(|e| e.to_string())?
        .time()
        .seconds())
}

/// Time-sorted revwalk over HEAD ancestry.
fn head_walk(repo: &git2::Repository) -> Result<git2::Revwalk<'_>, String> {
    let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
    walk.push_head().map_err(|e| e.to_string())?;
    walk.set_sorting(git2::Sort::TIME)
        .map_err(|e| e.to_string())?;
    Ok(walk)
}

/// Window size for default-branch scopes, in seconds.
const WINDOW_SECS: i64 = 36 * 60 * 60;

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Topic-branch scope: merge-base with the default branch.
fn topic_branch_base_in(
    dir: Option<&Path>,
    branch: &str,
    default_branch: &str,
) -> Result<(String, String), String> {
    let repo = open_repo(dir)?;

    Ok((
        repo.merge_base(
            commit_oid(&repo, "HEAD")?,
            commit_oid(&repo, default_branch)?,
        )
        .map_err(|_| format!("no common ancestor between {branch} and {default_branch}"))?
        .to_string(),
        format!("branch {branch}: changes since branching from {default_branch}"),
    ))
}

/// Resolve the diff base for an "MR scope": when on a topic branch, the
/// merge-base with the default branch; when committing straight onto the
/// default branch, its tip 36 hours ago.
pub fn mr_base() -> Result<(String, String), String> {
    mr_base_in(None)
}

/// Resolve the diff base for the current repository.
pub fn mr_base_in(dir: Option<&Path>) -> Result<(String, String), String> {
    let branch = current_branch_in(dir);
    let default = detect_default_branch_in(dir);
    let on_default = match (&branch, default) {
        (Some(b), Some(d)) => on_default_branch(b, d),
        _ => true,
    };

    if on_default {
        // The 36-hour window is the only scope contract on the default
        // branch: swapping in a sibling fork point would silently
        // resurface out-of-window files.
        return aged_default_base_in(dir);
    }

    if let Some(fp) = fork_point_in(dir)? {
        return Ok(fp);
    }
    fallback_base_in(dir, branch.as_deref(), default)
}

/// Base choice when no fork point exists: topic branches take their
/// merge-base, everything else the aged default window.
fn fallback_base_in(
    dir: Option<&Path>,
    branch: Option<&str>,
    default: Option<&str>,
) -> Result<(String, String), String> {
    match (branch, default) {
        (Some(b), Some(d)) if !on_default_branch(b, d) => topic_branch_base_in(dir, b, d),
        (_, Some(_)) => aged_default_base_in(dir),
        (Some(b), None) => Err(format!(
            "cannot determine a base for branch {b}: no master/main and \
             no sibling branches to fork from"
        )),
        _ => Err(
            "cannot find master/main (tried origin/main, origin/master, \
             main, master)"
                .to_string(),
        ),
    }
}

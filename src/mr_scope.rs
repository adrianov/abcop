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
use crate::git_changes::{
    current_branch_in, detect_default_branch_in, git_in, on_default_branch, rev_exists_in,
};

/// Default-branch scope: everything committed in the last 36 hours.
fn aged_default_base_in(
    dir: Option<&Path>,
    default_branch: &str,
) -> Result<(String, String), String> {
    let aged = format!("{default_branch}@{{36.hours.ago}}");
    if rev_exists_in(dir, &aged) {
        return Ok((aged, format!("last 36 hours on {default_branch}")));
    }
    Err(format!(
        "no commits in the last 36 hours on {default_branch}"
    ))
}

/// Topic-branch scope: merge-base with the default branch.
fn topic_branch_base_in(
    dir: Option<&Path>,
    branch: &str,
    default_branch: &str,
) -> Result<(String, String), String> {
    let mb = git_in(dir, &["merge-base", "HEAD", default_branch])?
        .trim()
        .to_string();
    if mb.is_empty() {
        return Err(format!(
            "git merge-base failed for {branch} vs {default_branch}"
        ));
    }
    Ok((
        mb,
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

    if let Some(fp) = fork_point_in(dir)? {
        // On the default branch the 36-hour window stays the primary
        // contract; a fork point is only a fallback there.
        if on_default {
            if let Ok(aged) = aged_default_base_in(dir, default.unwrap_or("origin/main")) {
                return Ok(aged);
            }
        }
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
        (_, Some(d)) => aged_default_base_in(dir, d),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::{self, sha_of};
    use std::path::Path;

    fn run_git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit_file(dir: &Path, name: &str, msg: &str) {
        std::fs::write(dir.join(name), format!("// {msg}\n")).unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-qm", msg]);
    }

    fn mr_base_in_dir(dir: &Path) -> Result<(String, String), String> {
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        let result = std::panic::catch_unwind(|| mr_base_in(None));
        std::env::set_current_dir(old).unwrap();
        result.unwrap()
    }

    /// Two stacked feature branches off main, with main advanced after the
    /// branch-offs; HEAD left on the outer branch. Returns feature/one's
    /// tip -- the nearest shared ancestor and expected base.
    fn seed_stacked_branches(dir: &Path) -> String {
        test_repo::seed_repo_with_base_commit(dir); // main @ a0
        run_git(dir, &["checkout", "-qb", "feature/one"]);
        commit_file(dir, "one.rb", "one"); // feature/one @ c1
        let c1 = sha_of(dir, "refs/heads/feature/one");
        run_git(dir, &["checkout", "-qb", "feature/two"]);
        commit_file(dir, "two.rb", "two"); // feature/two @ c2 (HEAD)
        // advance main AFTER branching: its tip must not become the base
        run_git(dir, &["checkout", "-q", "main"]);
        commit_file(dir, "main.rb", "advance main");
        run_git(dir, &["checkout", "-q", "feature/two"]);
        c1
    }

    #[test]
    fn mr_base_uses_nearest_parent_branch_fork_point() {
        let dir = test_repo::temp_dir("abcop_forkpoint");
        let c1 = seed_stacked_branches(&dir);
        let _guard = test_repo::cwd_lock();

        let (base, label) = mr_base_in_dir(&dir).expect("base resolves");
        assert_eq!(base, c1, "nearest fork point wins; label={label}");
    }

    #[test]
    fn mr_base_on_default_branch_keeps_36h_window() {
        let dir = test_repo::temp_dir("abcop_36h");
        let _guard = test_repo::cwd_lock();
        test_repo::seed_repo_with_base_commit(&dir);

        let (_base, label) = mr_base_in_dir(&dir).expect("base resolves");
        assert!(
            label.contains("36 hours"),
            "direct-main work keeps the 36h window; got {label}"
        );
    }
}

//! Integration contracts for [`crate::mr_scope`]: base-ref resolution is
//! driven against real throwaway git repositories so reflog, merge-base
//! and default-branch topologies are exercised with genuine git output.

use std::path::Path;

use crate::mr_scope::mr_base_in;
use crate::test_repo::{self, sha_of};

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

fn mr_base_in_dir(dir: &Path) -> Result<(String, String), String> {
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let result = std::panic::catch_unwind(|| mr_base_in(None));
    std::env::set_current_dir(old).unwrap();
    result.unwrap()
}

/// Commit with an explicit date so sibling-branch ordering inside
/// fork-point resolution is deterministic even within one wall-clock
/// second.
fn commit_file_at(dir: &Path, name: &str, msg: &str, date: &str) {
    std::fs::write(dir.join(name), format!("// {msg}\n")).unwrap();
    run_git(dir, &["add", "-A"]);
    let out = std::process::Command::new("git")
        .args(["commit", "-qm", msg])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dated commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Two stacked feature branches off main, with main advanced after the
/// branch-offs; HEAD left on the outer branch. Returns feature/one's
/// tip -- the nearest shared ancestor and expected base.
fn seed_stacked_branches(dir: &Path) -> String {
    test_repo::seed_repo_with_base_commit(dir); // main @ a0
    run_git(dir, &["checkout", "-qb", "feature/one"]);
    commit_file_at(dir, "one.rb", "one", "2025-01-01T00:01:00Z"); // feature/one @ c1
    let c1 = sha_of(dir, "refs/heads/feature/one");
    run_git(dir, &["checkout", "-qb", "feature/two"]);
    commit_file_at(dir, "two.rb", "two", "2025-01-01T00:02:00Z"); // feature/two @ c2 (HEAD)
    // advance main AFTER branching: its tip must not become the base
    run_git(dir, &["checkout", "-q", "main"]);
    commit_file_at(dir, "main.rb", "advance main", "2025-01-01T00:03:00Z");
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

/// Commit on top, optionally pinned to an explicit date; returns its sha.
fn commit_at(dir: &Path, name: &str, msg: &str, date: Option<&str>) -> String {
    std::fs::write(dir.join(name), format!("// {msg}\n")).unwrap();
    run_git(dir, &["add", "-A"]);
    let mut cmd = std::process::Command::new("git");
    cmd.args(["commit", "-qm", msg]).current_dir(dir);
    if let Some(d) = date {
        cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    sha_of(dir, "HEAD")
}

#[test]
fn mr_base_on_default_branch_rejects_empty_36h_window() {
    let dir = test_repo::temp_dir("abcop_36h_empty");
    let _guard = test_repo::cwd_lock();
    test_repo::seed_repo_with_base_commit(&dir);
    // Backdate the ONLY commit: zero window activity reachable from
    // HEAD must error even though refs exist.
    let out = std::process::Command::new("git")
        .args(["commit", "--amend", "-qm", "ancient"])
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "backdate failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let err = mr_base_in_dir(&dir).expect_err("empty window errors");
    assert!(
        err.contains("no commits in the last 36 hours"),
        "got: {err}"
    );
}

#[test]
fn mr_base_on_default_branch_anchors_at_window_edge() {
    let dir = test_repo::temp_dir("abcop_36h_anchor");
    let _guard = test_repo::cwd_lock();
    test_repo::seed_repo_with_base_commit(&dir);
    // Ancient history plus a fresh commit: the base must be the oldest
    // commit outside the window so only in-window content is diffed.
    let old = commit_at(&dir, "old.rb", "old work", Some("2020-01-01T00:00:00Z"));
    let _fresh = commit_at(&dir, "new.rb", "new work", None);

    let (base, label) = mr_base_in_dir(&dir).expect("base resolves");
    assert_eq!(base, old, "anchored at the window edge");
    assert!(label.contains("36 hours"), "got {label}");
}

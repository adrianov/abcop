//! Integration contracts for [`crate::mr_scope`]: base-ref resolution is
//! driven against real throwaway git repositories so merge-base and
//! default-branch topologies are exercised end to end. Fixtures are
//! built through `git2` — no external `git` process.

use std::path::Path;

use crate::mr_scope::mr_base_in;
use crate::test_repo::{self, sha_of};

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
fn commit_file_at(dir: &Path, name: &str, msg: &str, epoch: i64) {
    std::fs::write(dir.join(name), format!("// {msg}\n")).unwrap();
    test_repo::commit_all_at(dir, msg, epoch);
}

/// Two stacked feature branches off main, with main advanced after the
/// branch-offs; HEAD left on the outer branch. Returns feature/one's
/// tip -- the nearest shared ancestor and expected base.
fn seed_stacked_branches(dir: &Path) -> String {
    test_repo::seed_repo_with_base_commit(dir); // main @ a0
    test_repo::checkout_new_branch(dir, "feature/one");
    commit_file_at(dir, "one.rb", "one", 1_735_689_660); // 2025-01-01T00:01:00Z
    let c1 = sha_of(dir, "refs/heads/feature/one");
    test_repo::checkout_new_branch(dir, "feature/two");
    commit_file_at(dir, "two.rb", "two", 1_735_689_720); // 2025-01-01T00:02:00Z
    // advance main AFTER branching: its tip must not become the base
    test_repo::checkout_branch_named(dir, "main");
    commit_file_at(dir, "main.rb", "advance main", 1_735_689_780); // 2025-01-01T00:03:00Z
    test_repo::checkout_branch_named(dir, "feature/two");
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

/// Commit on top, optionally pinned to an explicit epoch; returns its sha.
fn commit_at(dir: &Path, name: &str, msg: &str, epoch: Option<i64>) -> String {
    std::fs::write(dir.join(name), format!("// {msg}\n")).unwrap();
    match epoch {
        Some(t) => test_repo::commit_all_at(dir, msg, t),
        None => test_repo::commit_all(dir, msg),
    }
    sha_of(dir, "HEAD")
}

#[test]
fn mr_base_on_default_branch_rejects_empty_36h_window() {
    let dir = test_repo::temp_dir("abcop_36h_empty");
    let _guard = test_repo::cwd_lock();
    test_repo::seed_repo_with_base_commit(&dir);
    // Backdate the ONLY commit: zero window activity reachable from
    // HEAD must error even though refs exist.
    test_repo::amend_head_at(&dir, "ancient", 1_577_836_800); // 2020-01-01

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
    let old = commit_at(&dir, "old.rb", "old work", Some(1_577_836_800));
    let _fresh = commit_at(&dir, "new.rb", "new work", None);

    let (base, label) = mr_base_in_dir(&dir).expect("base resolves");
    assert_eq!(base, old, "anchored at the window edge");
    assert!(label.contains("36 hours"), "got {label}");
}

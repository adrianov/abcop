//! Working-tree change-selection contracts, driven against real
//! throwaway repositories so libgit2 behaviour (stat freshness, hunk
//! ranges, untracked inclusion) is exercised end to end.

use std::path::Path;

use crate::git_changes::{Changeset, Lines};
use crate::test_repo;

/// Locks the contract behind MR/default scope: uncommitted work must be
/// selected no matter which of the three states it sits in.
#[test]
fn scope_includes_unstaged_staged_and_untracked_files() {
    let dir = test_repo::temp_dir("abcop_scope");
    let _guard = test_repo::cwd_lock();
    test_repo::seed_repo_with_base_commit(&dir);
    test_repo::stage_three_kinds_of_uncommitted_work(&dir);

    let cs = load_in_dir(&dir).expect("changeset loads");
    let seen = format!("{:?}", cs.files);
    assert!(
        matches!(cs.files.get("a.rb"), Some(Lines::Ranges(s)) if !s.is_empty()),
        "unstaged edit selected as ranges; saw {seen}"
    );
    assert!(
        matches!(cs.files.get("b.rb"), Some(Lines::Ranges(s)) if !s.is_empty()),
        "staged new file selected; saw {seen}"
    );
    assert!(
        matches!(cs.files.get("c.rb"), Some(Lines::All)),
        "untracked file counts fully; saw {seen}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn load_in_dir(dir: &Path) -> Result<Changeset, String> {
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let result = std::panic::catch_unwind(|| Changeset::load("HEAD"));
    std::env::set_current_dir(old).unwrap();
    result.unwrap()
}

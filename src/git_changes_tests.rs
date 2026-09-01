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

/// Files listed only in `$GIT_DIR/info/exclude` must not count as
/// uncommitted work. A gitfile workdir is the layout where a walk that
/// only reads `.gitignore` (and not `commondir`) used to leak them.
#[test]
fn scope_skips_info_exclude_on_gitfile_workdir() {
    let root = test_repo::temp_dir("abcop_gitfile_exclude");
    let _guard = test_repo::cwd_lock();
    let workdir = test_repo::seed_gitfile_exclude(&root);
    let cs = load_in_dir(&workdir).expect("changeset loads");
    let seen = format!("{:?}", cs.files);
    assert!(
        !cs.files.contains_key("AGENTS.md")
            && !cs.files.contains_key(".cursor/rules/ruby-style.mdc"),
        "info/exclude paths are not uncommitted work; saw {seen}"
    );
    assert!(
        matches!(cs.files.get("new.rb"), Some(Lines::All)),
        "real untracked file still counts fully; saw {seen}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn load_in_dir(dir: &Path) -> Result<Changeset, String> {
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let result = std::panic::catch_unwind(|| Changeset::load("HEAD"));
    std::env::set_current_dir(old).unwrap();
    result.unwrap()
}

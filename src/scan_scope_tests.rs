//! Scan-scope resolution contracts: which changeset each mode selects,
//! driven against real throwaway repositories. Resolution is cwd-based,
//! so every chdir assertion takes the shared working-directory lock.

use std::path::Path;

use crate::git_changes::Changeset;
use crate::scan_scope::resolve;
use crate::test_repo;

/// `--uncommitted` selects working-tree work only: unstaged edits, staged
/// files and untracked files are in; the branch's committed work is out.
#[test]
fn uncommitted_scope_excludes_committed_branch_work() {
    let dir = test_repo::temp_dir("abcop_uncommitted");
    let _guard = test_repo::cwd_lock();
    seed_branch_and_uncommitted_work(&dir);

    let cs = resolve_in_dir(&dir, false, true)
        .expect("scope resolves")
        .expect("changeset present");
    let seen = format!("{:?}", cs.files);
    assert!(
        cs.files.contains_key("a.rb"),
        "unstaged edit selected; saw {seen}"
    );
    assert!(
        cs.files.contains_key("b.rb"),
        "staged new file selected; saw {seen}"
    );
    assert!(
        cs.files.contains_key("c.rb"),
        "untracked file selected; saw {seen}"
    );
    assert!(
        !cs.files.contains_key("d.rb"),
        "committed branch work excluded; saw {seen}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Base commit, one committed branch file, and the three uncommitted
/// states seeded on top.
fn seed_branch_and_uncommitted_work(dir: &Path) {
    test_repo::seed_repo_with_base_commit(dir);
    std::fs::write(dir.join("d.rb"), "def committed\nend\n").unwrap();
    test_repo::commit_all(dir, "branch work");
    test_repo::stage_three_kinds_of_uncommitted_work(dir);
}

/// An explicitly requested scope must not silently widen: outside a
/// repository `--uncommitted` fails loudly instead of full-tree scanning.
#[test]
fn uncommitted_scope_fails_outside_a_repository() {
    let dir = test_repo::temp_dir("abcop_uncommitted_norepo");
    let _guard = test_repo::cwd_lock();
    let err = resolve_in_dir(&dir, false, true).expect_err("non-repository must fail");
    assert!(
        err.contains("no usable git repository"),
        "expected repository error; saw: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A dirty tree steers the bare default to uncommitted work only: the
/// three uncommitted states are in, committed branch work is out.
#[test]
fn default_prefers_uncommitted_work_when_the_tree_is_dirty() {
    let dir = test_repo::temp_dir("abcop_default_dirty");
    let _guard = test_repo::cwd_lock();
    seed_branch_and_uncommitted_work(&dir);

    let cs = resolve_in_dir(&dir, false, false)
        .expect("scope resolves")
        .expect("changeset present");
    let seen = format!("{:?}", cs.files);
    assert!(
        cs.files.contains_key("a.rb")
            && cs.files.contains_key("b.rb")
            && cs.files.contains_key("c.rb"),
        "uncommitted work selected; saw {seen}"
    );
    assert!(
        !cs.files.contains_key("d.rb"),
        "committed branch work excluded; saw {seen}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A clean tree falls through to the branch's MR scope: committed work
/// stays reviewed when nothing is uncommitted. Branch work is an
/// explicit topic branch off main, so the assertion is date-free.
#[test]
fn default_covers_branch_work_when_the_tree_is_clean() {
    let dir = test_repo::temp_dir("abcop_default_clean");
    let _guard = test_repo::cwd_lock();
    test_repo::seed_repo_with_base_commit(&dir);
    test_repo::checkout_new_branch(&dir, "feature/work");
    std::fs::write(dir.join("d.rb"), "def committed\nend\n").unwrap();
    test_repo::commit_all(&dir, "branch work");

    let cs = resolve_in_dir(&dir, false, false)
        .expect("scope resolves")
        .expect("changeset present");
    let seen = format!("{:?}", cs.files);
    assert!(
        cs.files.contains_key("d.rb"),
        "clean tree scans the branch scope; saw {seen}"
    );
    assert!(
        !cs.files.contains_key("a.rb"),
        "branch scope diffs the merge base, not the full tree; saw {seen}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Outside a repository the bare default still widens to the full tree
/// (only explicit narrow modes fail instead).
#[test]
fn default_falls_back_to_full_tree_outside_a_repository() {
    let dir = test_repo::temp_dir("abcop_default_norepo");
    let _guard = test_repo::cwd_lock();
    let cs = resolve_in_dir(&dir, false, false).expect("full-tree fallback is not an error");
    assert!(cs.is_none(), "no repository means no changeset; saw {cs:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn resolve_in_dir(dir: &Path, mr: bool, uncommitted: bool) -> Result<Option<Changeset>, String> {
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let result = std::panic::catch_unwind(|| resolve(mr, uncommitted, false, false, false));
    std::env::set_current_dir(old).unwrap();
    result.unwrap()
}

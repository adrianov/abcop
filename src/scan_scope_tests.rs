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

    let cs = resolve_in_dir(&dir)
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
    let err = resolve_in_dir(&dir).expect_err("non-repository must fail");
    assert!(
        err.contains("no usable git repository"),
        "expected repository error; saw: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn resolve_in_dir(dir: &Path) -> Result<Option<Changeset>, String> {
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let result = std::panic::catch_unwind(|| resolve(false, true, false, false, false));
    std::env::set_current_dir(old).unwrap();
    result.unwrap()
}

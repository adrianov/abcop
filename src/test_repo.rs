//! Shared fixtures for tests that drive a real `git` repository: temp
//! repo creation with deterministic layout, commit helpers, and the
//! process-wide working-directory lock every chdir-based assertion takes.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Tests that temporarily chdir into a fixture repository share one
/// process-wide working directory -- serialize them.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// A guard over [`CWD_LOCK`] immune to fixture-test panics held across
/// chdir: a poisoned lock still guards the process-wide cwd.
pub(crate) fn cwd_lock() -> MutexGuard<'static, ()> {
    match CWD_LOCK.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh nonexistent directory under the system temp dir, unique per
/// call so parallel test binaries never collide.
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("abcop_{tag}_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

pub(crate) fn sha_of(dir: &Path, rev: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "rev-parse {rev}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Empty repo whose first (and only) commit sits on `main`.
pub(crate) fn seed_repo_with_base_commit(dir: &Path) {
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "t@t"]);
    run_git(dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.rb"), "def base\nend\n").unwrap();
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-qm", "base"]);
    run_git(dir, &["branch", "-M", "main"]);
}

/// Stages everything and commits on the current branch.
pub(crate) fn commit_all(dir: &Path, msg: &str) {
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-qm", msg]);
}

/// One file edited (unstaged), one added (staged), one untracked.
pub(crate) fn stage_three_kinds_of_uncommitted_work(dir: &Path) {
    std::fs::write(dir.join("a.rb"), "def base\n  x = 1\nend\n").unwrap();
    std::fs::write(dir.join("b.rb"), "def staged_new\nend\n").unwrap();
    run_git(dir, &["add", "b.rb"]);
    std::fs::write(dir.join("c.rb"), "def untracked_new\nend\n").unwrap();
}

//! Shared fixtures for tests that drive a real git repository: temp
//! repo creation with deterministic layout and commit helpers. Every
//! repository mutation goes through the built-in `git2` binding — no
//! external `git` process is spawned, matching production analyse paths.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use git2::{Repository, Signature, Time};

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

fn open(dir: &Path) -> Repository {
    Repository::open(dir).expect("open fixture repo")
}

fn sig(when: Option<i64>) -> Signature<'static> {
    let time = Time::new(when.unwrap_or_else(now_secs), 0);
    Signature::new("t", "t@t", &time).expect("signature")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Resolves a rev to its object id hex string.
pub(crate) fn sha_of(dir: &Path, rev: &str) -> String {
    open(dir)
        .revparse_single(rev)
        .expect("revparse")
        .id()
        .to_string()
}

/// Empty repo whose first (and only) commit sits on `main`.
pub(crate) fn seed_repo_with_base_commit(dir: &Path) {
    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = Repository::init_opts(dir, &opts).expect("init");
    std::fs::write(dir.join("a.rb"), "def base\nend\n").unwrap();
    commit_tree(&repo, "base", None);
}

/// Stages everything and commits on the current branch.
pub(crate) fn commit_all(dir: &Path, msg: &str) {
    commit_tree(&open(dir), msg, None);
}

/// Stages everything and commits with an explicit unix author/committer
/// timestamp so sibling-branch ordering stays deterministic.
pub(crate) fn commit_all_at(dir: &Path, msg: &str, epoch: i64) {
    commit_tree(&open(dir), msg, Some(epoch));
}

/// Rewrites HEAD's commit message and timestamps in place (amend).
pub(crate) fn amend_head_at(dir: &Path, msg: &str, epoch: i64) {
    let repo = open(dir);
    let head = repo.head().expect("head").peel_to_commit().expect("commit");
    let sig = sig(Some(epoch));
    head.amend(Some("HEAD"), Some(&sig), Some(&sig), None, Some(msg), None)
        .expect("amend");
}

/// Creates and checks out a topic branch off the current HEAD.
pub(crate) fn checkout_new_branch(dir: &Path, name: &str) {
    let repo = open(dir);
    let commit = repo.head().expect("head").peel_to_commit().expect("commit");
    repo.branch(name, &commit, false).expect("branch");
    checkout_branch(&repo, name);
}

/// Checks out an existing local branch by short name.
pub(crate) fn checkout_branch_named(dir: &Path, name: &str) {
    checkout_branch(&open(dir), name);
}

/// One file edited (unstaged), one added (staged), one untracked.
pub(crate) fn stage_three_kinds_of_uncommitted_work(dir: &Path) {
    std::fs::write(dir.join("a.rb"), "def base\n  x = 1\nend\n").unwrap();
    std::fs::write(dir.join("b.rb"), "def staged_new\nend\n").unwrap();
    stage_paths(&open(dir), &["b.rb"]);
    std::fs::write(dir.join("c.rb"), "def untracked_new\nend\n").unwrap();
}

fn checkout_branch(repo: &Repository, name: &str) {
    let branch = repo
        .find_branch(name, git2::BranchType::Local)
        .expect("find branch");
    let reference = branch.into_reference();
    let treeish = reference.peel_to_tree().expect("tree");
    repo.checkout_tree(treeish.as_object(), None)
        .expect("checkout tree");
    repo.set_head(reference.name().expect("ref name"))
        .expect("set head");
}

fn stage_paths(repo: &Repository, paths: &[&str]) {
    let mut idx = repo.index().expect("index");
    for p in paths {
        idx.add_path(Path::new(p)).expect("add path");
    }
    idx.write().expect("write index");
}

fn stage_all(repo: &Repository) -> git2::Tree<'_> {
    let mut idx = repo.index().expect("index");
    idx.add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .expect("add all");
    idx.write().expect("write index");
    let oid = idx.write_tree().expect("write tree");
    repo.find_tree(oid).expect("find tree")
}

fn head_parent(repo: &Repository) -> Option<git2::Commit<'_>> {
    repo.head().ok()?.peel_to_commit().ok()
}

fn commit_tree(repo: &Repository, msg: &str, when: Option<i64>) {
    let tree = stage_all(repo);
    let sig = sig(when);
    let parent = head_parent(repo);
    let parents: Vec<_> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
        .expect("commit");
}


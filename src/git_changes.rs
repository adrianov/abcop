//! Git working-tree change detection for `--changed` mode.
//!
//! Semantics mirror refactor_gpt quality gates: compare the working tree
//! against a base ref (`HEAD` by default) with `git diff -U0 -W`: the
//! function-context option expands every hunk to the full enclosing
//! function, so a hunk range IS a touched function body. New-side line
//! numbers are collected from `@@` headers, plus untracked files
//! (`ls-files --others --exclude-standard`), which count as fully changed.
//!
//! Unified-diff parsing itself lives in [`crate::diffparse`]; base-ref
//! resolution lives in [`crate::mr_scope`].
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use crate::diffparse::parse_diff;

pub fn git(args: &[&str]) -> Result<String, String> {
    git_in(None, args)
}

/// Runs git optionally inside `dir` -- MR-scope resolution takes a
/// directory so tests can exercise repository topologies without
/// mutating the process-wide working directory.
pub(crate) fn git_in(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Which lines of a changed file were touched.
#[derive(Debug)]
pub enum Lines {
    /// untracked/new file: every line counts
    All,
    Ranges(BTreeSet<usize>),
}

#[derive(Debug)]
pub struct Changeset {
    pub root: String,
    pub files: BTreeMap<String, Lines>,
}

impl Changeset {
    pub fn load(base: &str) -> Result<Changeset, String> {
        let root = git(&["rev-parse", "--show-toplevel"])?
            .trim()
            .replace('\\', "/");
        let mut files = BTreeMap::new();
        parse_diff(&git(&["diff", "-U0", base])?, &mut files);
        add_untracked(&mut files);
        Ok(Changeset { root, files })
    }

    pub fn line_selected(&self, rel: &str, line: usize) -> bool {
        match self.files.get(rel) {
            None => false,
            Some(Lines::All) => true,
            Some(Lines::Ranges(set)) => set.contains(&line),
        }
    }

    /// True when any changed line falls inside `[start, end]`.
    pub fn span_selected(&self, rel: &str, start: usize, end: usize) -> bool {
        match self.files.get(rel) {
            None => false,
            Some(Lines::All) => true,
            Some(Lines::Ranges(set)) => set
                .range(start..=end)
                .next()
                .map(|l| *l <= end)
                .unwrap_or(false),
        }
    }

    /// Repo-relative path of an absolute or already-relative path.
    pub fn rel_of<'a>(&'a self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("{}/", self.root);
        path.strip_prefix(&prefix)
            .or_else(|| (path == self.root).then_some(""))
    }

    /// Changed code files that still exist on disk, as absolute paths.
    pub fn code_files(&self) -> Vec<std::path::PathBuf> {
        self.files
            .keys()
            .filter(|k| crate::paths::is_code_path(Path::new(k)))
            .map(|k| Path::new(&self.root).join(k))
            .filter(|p| p.exists())
            .collect()
    }
}

fn add_untracked(files: &mut BTreeMap<String, Lines>) {
    let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"]).unwrap_or_default();
    for f in untracked.split('\0').filter(|s| !s.is_empty()) {
        files.insert(crate::diffparse::normalize(f), Lines::All);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            matches!(cs.files.get("a.rb"), Some(Lines::Ranges(s)) if !s.is_empty()),
            "unstaged edit selected as ranges"
        );
        assert!(
            matches!(cs.files.get("b.rb"), Some(Lines::Ranges(s)) if !s.is_empty()),
            "staged new file selected"
        );
        assert!(
            matches!(cs.files.get("c.rb"), Some(Lines::All)),
            "untracked file counts fully"
        );
    }

    fn load_in_dir(dir: &Path) -> Result<Changeset, String> {
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        let result = std::panic::catch_unwind(|| Changeset::load("HEAD"));
        std::env::set_current_dir(old).unwrap();
        result.unwrap()
    }
}

const DEFAULT_BRANCHES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

pub(crate) fn rev_exists_in(dir: Option<&Path>, rev: &str) -> bool {
    git_in(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .is_ok()
}

pub(crate) fn current_branch_in(dir: Option<&Path>) -> Option<String> {
    let b = git_in(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let t = b.trim();
    (!t.is_empty() && t != "HEAD").then(|| t.to_string())
}

pub(crate) fn detect_default_branch_in(dir: Option<&Path>) -> Option<&'static str> {
    DEFAULT_BRANCHES
        .iter()
        .copied()
        .find(|c| rev_exists_in(dir, c))
}

pub(crate) fn is_ancestor_in(dir: Option<&Path>, a: &str, b: &str) -> bool {
    git_in(dir, &["merge-base", "--is-ancestor", a, b]).is_ok()
}

/// HEAD detached or sitting on the default branch under either spelling.
pub(crate) fn on_default_branch(branch: &str, default_branch: &str) -> bool {
    if branch == "HEAD" {
        return true;
    }
    let bare = default_branch
        .strip_prefix("origin/")
        .unwrap_or(default_branch);
    branch == bare || branch == default_branch
}

//! Git working-tree change detection for `--changed` mode.
//!
//! Semantics mirror refactor_gpt quality gates: compare the working tree
//! against a base ref (`HEAD` by default) with `git diff -U0 -W`: the
//! function-context option expands every hunk to the full enclosing
//! function, so a hunk range IS a touched function body. New-side line
//! numbers are collected from `@@` headers, plus untracked files
//! (`ls-files --others --exclude-standard`), which count as fully changed.
//! Unified-diff parsing itself lives in [`crate::diffparse`].

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use crate::diffparse::parse_diff;

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

pub fn git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
        use std::path::Path;
        self.files
            .keys()
            .filter(|k| crate::paths::is_code_path(std::path::Path::new(k)))
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

const DEFAULT_BRANCHES: [&str; 4] = ["origin/main", "origin/master", "main", "master"];

fn rev_exists(rev: &str) -> bool {
    git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{rev}^{{commit}}"),
    ])
    .is_ok()
}

pub fn current_branch() -> Option<String> {
    let b = git(&["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let t = b.trim();
    (!t.is_empty() && t != "HEAD").then(|| t.to_string())
}

fn detect_default_branch() -> Option<&'static str> {
    DEFAULT_BRANCHES.iter().copied().find(|c| rev_exists(c))
}

/// Resolve the diff base for an "MR scope": when on a topic branch, the
/// merge-base with the default branch; when committing straight onto the
/// default branch, its tip 36 hours ago.
pub fn mr_base() -> Result<(String, String), String> {
    let default_branch = detect_default_branch().ok_or_else(missing_default_branch)?;
    let branch = current_branch().unwrap_or_else(|| "HEAD".to_string());
    if on_default_branch(&branch, default_branch) {
        return aged_default_base(default_branch);
    }
    topic_branch_base(&branch, default_branch)
}

fn missing_default_branch() -> String {
    "cannot find master/main (tried origin/main, origin/master, main, \
     master); pass --base"
        .to_string()
}

/// HEAD detached or sitting on the default branch under either spelling.
fn on_default_branch(branch: &str, default_branch: &str) -> bool {
    if branch == "HEAD" {
        return true;
    }
    let bare = default_branch
        .strip_prefix("origin/")
        .unwrap_or(default_branch);
    branch == bare || branch == default_branch
}

/// Default-branch scope: everything committed in the last 36 hours.
fn aged_default_base(default_branch: &str) -> Result<(String, String), String> {
    let aged = format!("{default_branch}@{{36.hours.ago}}");
    if rev_exists(&aged) {
        return Ok((aged, format!("last 36 hours on {default_branch}")));
    }
    Err(format!(
        "no commits in the last 36 hours on {default_branch}; \
         pass --base <ref> or use --changed"
    ))
}

/// Topic-branch scope: merge-base with the default branch.
fn topic_branch_base(branch: &str, default_branch: &str) -> Result<(String, String), String> {
    let mb = git(&["merge-base", "HEAD", default_branch])?
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    /// Locks the contract behind MR/default scope: uncommitted work must be
    /// selected no matter which of the three states it sits in -- unstaged
    /// edit of a tracked file, staged new file, or fully untracked file.
    /// This is the spec the gix migration must preserve.
    #[test]
    fn scope_includes_unstaged_staged_and_untracked_files() {
        let dir = std::env::temp_dir().join(format!("abcop_scope_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        seed_repo_with_base_commit(&dir);
        stage_three_kinds_of_uncommitted_work(&dir);

        let cs = load_in_dir(&dir).expect("changeset loads");
        let _ = std::fs::remove_dir_all(&dir);
        assert_scope(cs);
    }

    /// `Changeset::load` resolves against the current directory, so run it
    /// from inside the temp repo and restore the original cwd afterwards.
    fn load_in_dir(dir: &Path) -> Result<Changeset, String> {
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        let result = std::panic::catch_unwind(|| Changeset::load("HEAD"));
        std::env::set_current_dir(old).unwrap();
        result.unwrap()
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
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

    fn seed_repo_with_base_commit(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "t@t"]);
        run_git(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("a.rb"), "def base\nend\n").unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-qm", "base"]);
    }

    /// One uncommitted change per state: unstaged edit, staged new file,
    /// fully untracked file.
    fn stage_three_kinds_of_uncommitted_work(dir: &Path) {
        // unstaged modification of a tracked file
        std::fs::write(dir.join("a.rb"), "def base\n  x = 1\nend\n").unwrap();
        // staged brand-new file
        std::fs::write(dir.join("b.rb"), "def staged_new\nend\n").unwrap();
        run_git(dir, &["add", "b.rb"]);
        // fully untracked file
        std::fs::write(dir.join("c.rb"), "def untracked_new\nend\n").unwrap();
    }

    fn assert_scope(cs: Changeset) {
        assert!(
            matches!(cs.files.get("a.rb"), Some(Lines::Ranges(s)) if !s.is_empty()),
            "unstaged edit selected as ranges"
        );
        // staged new file is not untracked: it arrives via the diff as
        // ranges covering its added lines
        assert!(
            matches!(cs.files.get("b.rb"), Some(Lines::Ranges(s)) if !s.is_empty()),
            "staged new file selected"
        );
        assert!(
            matches!(cs.files.get("c.rb"), Some(Lines::All)),
            "untracked file counts fully"
        );
    }
}

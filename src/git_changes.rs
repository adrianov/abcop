//! Working-tree change detection: the `Changeset` domain records which
//! lines of which files a base ref differs from the current worktree.
//!
//! Everything runs through the built-in `git2` binding (libgit2): no
//! external `git` process is ever spawned at runtime. Semantics mirror
//! refactor_gpt quality gates: compare the working tree against a base
//! ref with zero context lines, so a recorded line range IS a touched
//! line stretch; untracked files count as fully changed (see
//! [`crate::untracked_scan`]); test fixtures still seed their repos
//! through the real git CLI, which is setup, not product behaviour.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use git2::{DiffOptions, Repository};

use crate::repo_state::open_repo;
use crate::untracked_scan::add_untracked;

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
        let repo = open_repo(None)?;
        let root = repo
            .workdir()
            .and_then(|p| p.to_str())
            .ok_or("repository has no work directory")?
            .trim_end_matches('/')
            .replace('\\', "/");
        let files = changed_lines(&repo, base)?;
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

/// New-side line selection per file for `base -> working tree`.
///
/// Tracked changes come from a context-less tree-to-worktree diff whose
/// hunks feed `start..start+max(count,1)` -- the same union a `-U0`
/// unified-diff parse produced before. Deletions contribute nothing,
/// typechanges are ignored, staged additions surface as full files, and
/// every remaining untracked-and-not-ignored file counts fully.
fn changed_lines(repo: &Repository, base: &str) -> Result<BTreeMap<String, Lines>, String> {
    let tree = base_tree(repo, base)?;
    let mut files: BTreeMap<String, Lines> = tracked_ranges(repo, &tree)?
        .into_iter()
        .map(|(p, set)| (p, Lines::Ranges(set)))
        .collect();
    add_untracked(repo, &mut files)?;
    Ok(files)
}

/// The tree an MR-style diff runs against.
fn base_tree<'r>(repo: &'r Repository, base: &str) -> Result<git2::Tree<'r>, String> {
    repo.revparse_single(base)
        .map_err(|e| format!("cannot resolve base {base}: {e}"))?
        .peel_to_commit()
        .map_err(|e| format!("base {base} is not a commit: {e}"))?
        .tree()
        .map_err(|e| e.to_string())
}

/// New-side hunk ranges per tracked file for the tree-to-worktree diff.
fn tracked_ranges(
    repo: &Repository,
    tree: &git2::Tree<'_>,
) -> Result<Vec<(String, BTreeSet<usize>)>, String> {
    sync_index_stats(repo);
    let mut opts = DiffOptions::new();
    opts.context_lines(0);

    let mut sets: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    diff_into(repo, tree, &mut opts, &mut sets)?;
    Ok(sets.into_iter().collect())
}

/// Runs the tree-to-worktree-with-index diff, filling `sets`.
fn diff_into(
    repo: &Repository,
    tree: &git2::Tree<'_>,
    opts: &mut DiffOptions,
    sets: &mut BTreeMap<String, BTreeSet<usize>>,
) -> Result<(), String> {
    repo.diff_tree_to_workdir_with_index(Some(tree), Some(opts))
        .map_err(|e| e.to_string())?
        .foreach(
            &mut |_, _| true,
            None,
            Some(&mut |d, h| {
                on_hunk(d, h, sets);
                true
            }),
            None,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Synchronises index stat data with the working tree. Fresh git
/// processes re-hash racily-clean files (mtime within stat
/// granularity); libgit2 keeps trusting its cache, so just-written
/// edits would otherwise read as unmodified. In-memory only: no
/// `index.write()`, so the user's staging area stays untouched.
fn sync_index_stats(repo: &Repository) {
    if let Ok(mut idx) = repo.index() {
        let _ = idx.update_all(["*"], None::<&mut git2::IndexMatchedPath>);
    }
}

/// Records the hunk under its own delta's new-side path: tracked adds
/// and modifications contribute `start..start+max(count,1)` -- the same
/// union a `-U0` parse produced.
fn on_hunk(
    delta: git2::DiffDelta,
    hunk: git2::DiffHunk,
    sets: &mut BTreeMap<String, BTreeSet<usize>>,
) {
    if !matches!(delta.status(), git2::Delta::Modified | git2::Delta::Added) {
        return;
    }
    let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) else {
        return;
    };
    let start = hunk.new_start() as usize;
    sets.entry(path.to_string())
        .or_default()
        .extend(start..start + (hunk.new_lines() as usize).max(1));
}

//! Untracked work-file detection: which files exist in the working
//! directory outside git's knowledge. The `ignore` crate walks the tree
//! honouring `.gitignore`-style filters; anything the index already
//! knows about is subtracted, so only genuinely untracked paths remain.

use std::collections::BTreeMap;
use std::path::Path;

use git2::Repository;

use crate::git_changes::Lines;

/// Untracked, not-ignored work files count as fully changed; they only
/// fill paths the tracked-diff did not claim.
pub(crate) fn add_untracked(
    repo: &Repository,
    files: &mut BTreeMap<String, Lines>,
) -> Result<(), String> {
    for rel in untracked_rel_paths(repo) {
        if !files.contains_key(&rel) {
            files.insert(rel, Lines::All);
        }
    }
    Ok(())
}

/// Workdir-relative paths of untracked, not-ignored files: everything
/// the walker finds minus every path the index knows about.
fn untracked_rel_paths(repo: &Repository) -> Vec<String> {
    let Some(root) = repo.workdir() else {
        return Vec::new();
    };
    let tracked = index_paths(repo);
    untracked_walker(root)
        .filter_map(Result::ok)
        .filter_map(|e| untracked_rel_of(&e, root, &tracked))
        .collect()
}

/// Workdir-relative path when the entry is a regular file outside the
/// index; `None` for directories, tracked files and unreadable paths.
fn untracked_rel_of(
    entry: &ignore::DirEntry,
    root: &Path,
    tracked: &std::collections::HashSet<Vec<u8>>,
) -> Option<String> {
    if !entry.file_type()?.is_file() {
        return None;
    }
    let rel = normalize(entry.path().strip_prefix(root).ok()?.to_str()?);
    (!tracked.contains(rel.as_bytes())).then_some(rel)
}

/// Walk builder for workdir contents: dotfiles are untracked candidates
/// like any other, but the git store itself never is.
fn untracked_walker(root: &Path) -> ignore::Walk {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .standard_filters(true)
        .hidden(false)
        .filter_entry(|e| e.file_name() != std::ffi::OsStr::new(".git"));
    builder.build()
}

/// Byte paths known to the index -- the tracked-universe reference set.
fn index_paths(repo: &Repository) -> std::collections::HashSet<Vec<u8>> {
    repo.index()
        .map(|idx| idx.iter().map(|e| e.path.to_vec()).collect())
        .unwrap_or_default()
}

/// Repo-style path separators on any platform.
fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

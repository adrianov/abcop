//! Untracked work-file detection: which files exist in the working
//! directory outside git's knowledge. libgit2 status is the source of
//! truth so `.gitignore`, `$GIT_DIR/info/exclude`, and the global
//! excludes file all apply — including submodule / gitfile layouts
//! where the `ignore` crate never loads `info/exclude`.

use std::collections::BTreeMap;

use git2::{Repository, StatusOptions, Statuses};

use crate::git_changes::Lines;

/// Untracked, not-ignored work files count as fully changed; they only
/// fill paths the tracked-diff did not claim.
pub(crate) fn add_untracked(
    repo: &Repository,
    files: &mut BTreeMap<String, Lines>,
) -> Result<(), String> {
    for rel in untracked_rel_paths(repo)? {
        files.entry(rel).or_insert(Lines::All);
    }
    Ok(())
}

/// Workdir-relative paths of untracked, not-ignored files, matching
/// `git status --porcelain` (no ignored paths).
fn untracked_rel_paths(repo: &Repository) -> Result<Vec<String>, String> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    Ok(wt_new_paths(
        &repo.statuses(Some(&mut opts)).map_err(|e| e.to_string())?,
    ))
}

fn wt_new_paths(statuses: &Statuses<'_>) -> Vec<String> {
    statuses
        .iter()
        .filter(|s| s.status().is_wt_new())
        .filter_map(|s| s.path().map(|p| p.replace('\\', "/")))
        .collect()
}

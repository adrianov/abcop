//! HEAD blob and its cached unit inventory.
//!
//! A matching blob id means the worktree text is unchanged. Otherwise the
//! blob is read once and its units are reused for later scans of the same
//! commit.

use std::path::{Path, PathBuf};

use crate::abc::{Limits, Unit};

pub(super) enum Head {
    Same,
    Diff { oid: git2::Oid, bytes: Vec<u8> },
}

pub(super) fn load_head(path: &Path, src: &[u8]) -> Option<Head> {
    let repo = discover(path)?;
    let oid = head_blob(&repo, &work_rel(&repo, path)?)?;
    if blob_id(src) == Some(oid) {
        return Some(Head::Same);
    }
    let bytes = repo.find_blob(oid).ok()?.content().to_vec();
    if bytes == src {
        return Some(Head::Same);
    }
    Some(Head::Diff { oid, bytes })
}

pub(super) struct Baseline {
    pub units: Vec<Unit>,
    pub module_score: f64,
}

pub(super) fn head_baseline(
    cache: Option<&crate::cache::Cache>,
    path: &Path,
    oid: git2::Oid,
    src: &[u8],
) -> Baseline {
    if let Some(hit) = cache.and_then(|c| c.get(&c.blob_key(path, &oid))) {
        return Baseline {
            units: hit.units,
            module_score: hit.module_score,
        };
    }
    let found = inventory(path, src);
    if let Some(c) = cache {
        c.store(
            &c.blob_key(path, &oid),
            &crate::cache::CachedDiags {
                abc: Vec::new(),
                used_once: Vec::new(),
                never_used: Vec::new(),
                module_abc: None,
                units: found.units.clone(),
                module_score: found.module_score,
            },
        );
    }
    found
}

fn blob_id(src: &[u8]) -> Option<git2::Oid> {
    git2::Oid::hash_object(git2::ObjectType::Blob, src).ok()
}

fn discover(path: &Path) -> Option<git2::Repository> {
    let start = if path.exists() { path } else { path.parent()? };
    git2::Repository::discover(start).ok()
}

fn work_rel(repo: &git2::Repository, path: &Path) -> Option<PathBuf> {
    let root = std::fs::canonicalize(repo.workdir()?).ok()?;
    std::fs::canonicalize(path)
        .ok()?
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_path_buf())
}

fn head_blob(repo: &git2::Repository, rel: &Path) -> Option<git2::Oid> {
    let tree = repo.head().ok()?.peel_to_tree().ok()?;
    Some(tree.get_path(rel).ok()?.id())
}

/// Parse `src` once. `increased` stays off so this does not re-enter the filter.
fn inventory(path: &Path, src: &[u8]) -> Baseline {
    let mut r = super::blank_with(path);
    let _ = super::analyze_src_into(
        &mut r,
        path,
        src,
        Some("abc"),
        Limits {
            method: 0.0,
            module: f64::MAX,
            increased: false,
        },
    );
    Baseline {
        units: r.units,
        module_score: r.module_score,
    }
}

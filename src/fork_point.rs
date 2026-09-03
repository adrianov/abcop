//! Fork-point detection: where the current branch last shared history
//! with a sibling. One multi-base merge pass replaces per-branch probes,
//! so detection cost stays flat no matter how many branches the
//! repository carries. All repository access goes through the built-in
//! `git2` binding; no external git process is spawned.

use std::collections::HashMap;
use std::path::Path;

use crate::repo_state::{commit_oid, current_branch_in, is_ancestor_in, open_repo};

/// Newest commit shared by HEAD and any sibling branch tip -- the
/// parent/fork point.
pub(super) fn fork_point_in(dir: Option<&Path>) -> Result<Option<(String, String)>, String> {
    let repo = open_repo(dir)?;
    let head = commit_oid(&repo, "HEAD")?.to_string();

    let Some((tips, names)) = sibling_tips_in(&repo, &head, current_branch_in(dir).as_deref())?
    else {
        return Ok(None);
    };
    pick_fork_point(&repo, &head, &tips).map(|best| best.map(|b| fork_label(&b, &names)))
}

/// Deepest shared ancestor of HEAD across `tips` by committer date.
fn pick_fork_point(
    repo: &git2::Repository,
    head: &str,
    tips: &[String],
) -> Result<Option<String>, String> {
    let mut best: Option<(i64, String)> = None;
    for b in boundary_bases_in(repo, head, tips)? {
        let ts = commit_ts(repo, &b);
        if best.as_ref().is_none_or(|(t, _)| ts > *t) {
            best = Some((ts, b));
        }
    }
    Ok(best.map(|(_, b)| b))
}

fn commit_ts(repo: &git2::Repository, sha: &str) -> i64 {
    git2::Oid::from_str(sha)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
        .map(|c| c.time().seconds())
        .unwrap_or(i64::MIN)
}

fn fork_label(sha: &str, names: &HashMap<String, String>) -> (String, String) {
    (
        sha.into(),
        names
        .get(sha)
        .map(|p| format!("changes since fork point (parent {p})"))
            .unwrap_or_else(|| "changes since fork point".into()),
    )
}

/// Distinct sibling tips (own branch excluded) plus a display name per
/// sha (`heads/` preferred over `remotes/`). `None` when alone.
fn sibling_tips_in(
    repo: &git2::Repository,
    head: &str,
    branch: Option<&str>,
) -> Result<Option<(Vec<String>, HashMap<String, String>)>, String> {
    let mut tips = Vec::new();
    let mut names = HashMap::new();
    for glob in ["refs/heads/*", "refs/remotes/*"] {
        collect_glob(repo, glob, head, branch, &mut tips, &mut names)?;
    }
    Ok((!tips.is_empty()).then_some((tips, names)))
}

fn collect_glob(
    repo: &git2::Repository,
    glob: &str,
    head: &str,
    branch: Option<&str>,
    tips: &mut Vec<String>,
    names: &mut HashMap<String, String>,
) -> Result<(), String> {
    for r in repo.references_glob(glob).map_err(|e| e.to_string())? {
        let Some((sha, refname)) = tip_of(r.map_err(|e| e.to_string())?)? else {
            continue;
        };
        if sha == head || is_own_ref(branch, &refname) {
            continue;
        }
        record_tip(tips, names, sha, refname);
    }
    Ok(())
}

fn tip_of(r: git2::Reference<'_>) -> Result<Option<(String, String)>, String> {
    let Some(name) = r.name().map(str::to_string) else {
        return Ok(None);
    };

    Ok(r.resolve()
        .map_err(|e| e.to_string())?
        .target()
        .map(|o| (o.to_string(), name)))
}

fn record_tip(
    tips: &mut Vec<String>,
    names: &mut HashMap<String, String>,
    sha: String,
    refname: String,
) {
    if !tips.iter().any(|t| t == &sha) {
        tips.push(sha.clone());
    }
    let display = refname
        .trim_start_matches("refs/heads/")
        .trim_start_matches("refs/remotes/");
    names.entry(sha).or_insert_with(|| display.to_string());
}

fn is_own_ref(branch: Option<&str>, refname: &str) -> bool {
    let Some(b) = branch else {
        return false;
    };
    refname
        .strip_prefix("refs/heads/")
        .or_else(|| refname.strip_prefix("refs/remotes/"))
        == Some(b)
}

/// Shared-ancestor frontier of HEAD across tips; drop bases that are
/// proper ancestors of another so the deepest fork point wins.
fn boundary_bases_in(
    repo: &git2::Repository,
    head: &str,
    tips: &[String],
) -> Result<Vec<String>, String> {
    let mut set = Vec::with_capacity(tips.len() + 1);
    for rev in std::iter::once(head).chain(tips.iter().map(String::as_str)) {
        set.push(commit_oid(repo, rev)?);
    }
    Ok(drop_ancestor_bases(
        repo.workdir(),
        merge_base_shas(repo, &set),
    ))
}

fn merge_base_shas(repo: &git2::Repository, set: &[git2::Oid]) -> Vec<String> {
    match repo.merge_bases_many(set) {
        Ok(arr) => arr.iter().map(|o| o.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn drop_ancestor_bases(dir: Option<&Path>, mut bases: Vec<String>) -> Vec<String> {
    let stale: Vec<usize> = (0..bases.len())
        .filter(|&i| (0..bases.len()).any(|j| i != j && is_ancestor_in(dir, &bases[i], &bases[j])))
        .collect();
    for i in stale.into_iter().rev() {
        bases.remove(i);
    }
    bases
}

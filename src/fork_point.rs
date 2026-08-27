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
    let head = commit_oid(&repo, "HEAD")?;
    let branch = current_branch_in(dir);
    let Some((tips, names)) = sibling_tips_in(&repo, &head.to_string(), branch.as_deref())? else {
        return Ok(None);
    };
    pick_fork_point(&repo, &head.to_string(), &tips)
        .map(|best| best.map(|b| fork_label(&b, &names)))
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

/// Committer epoch of a hex sha, smallest representable when unresolvable.
fn commit_ts(repo: &git2::Repository, sha: &str) -> i64 {
    git2::Oid::from_str(sha)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
        .map(|c| c.time().seconds())
        .unwrap_or(i64::MIN)
}

/// Scan label for the chosen fork point, naming the sibling branch that
/// provided it when one is known.
fn fork_label(sha: &str, names: &HashMap<String, String>) -> (String, String) {
    match names.get(sha) {
        Some(parent) => (
            sha.to_string(),
            format!("changes since fork point (parent {parent})"),
        ),
        None => (sha.to_string(), "changes since fork point".to_string()),
    }
}

/// Distinct sibling tips (own branch excluded, one entry per sha) plus
fn ref_tips(repo: &git2::Repository) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for glob in ["refs/heads/*", "refs/remotes/*"] {
        collect_glob(repo, glob, &mut out)?;
    }
    Ok(out)
}

/// Appends `(oid hex, refname)` for every reference matching `glob`.
fn collect_glob(
    repo: &git2::Repository,
    glob: &str,
    out: &mut Vec<(String, String)>,
) -> Result<(), String> {
    for r in repo.references_glob(glob).map_err(|e| e.to_string())? {
        if let Some(tip) = tip_of(r.map_err(|e| e.to_string())?)? {
            out.push(tip);
        }
    }
    Ok(())
}

/// `(oid hex, refname)` of a direct reference, skipping unnamed ones.
fn tip_of(r: git2::Reference<'_>) -> Result<Option<(String, String)>, String> {
    let Some(name) = r.name().map(str::to_string) else {
        return Ok(None);
    };
    let oid = r.resolve().map_err(|e| e.to_string())?.target();
    Ok(oid.map(|o| (o.to_string(), name)))
}

/// Distinct sibling tips (own branch excluded, one entry per sha) plus
/// a display-name per sha, preferring the `heads/` spelling over the
/// `remotes/` alias. `None` when no other branch exists.
fn sibling_tips_in(
    repo: &git2::Repository,
    head: &str,
    branch: Option<&str>,
) -> Result<Option<(Vec<String>, HashMap<String, String>)>, String> {
    let mut tips: Vec<String> = Vec::new();
    let mut names_by_sha: HashMap<String, String> = HashMap::new();
    for (sha, refname) in ref_tips(repo)? {
        if excluded_tip(&sha, head, branch, &refname) {
            continue;
        }
        record_tip(&mut tips, &mut names_by_sha, sha, refname);
    }
    Ok((!tips.is_empty()).then_some((tips, names_by_sha)))
}

/// Own-branch tips (both spellings) and HEAD itself never count as
/// siblings.
fn excluded_tip(sha: &str, head: &str, branch: Option<&str>, refname: &str) -> bool {
    sha == head || is_own_ref(branch, refname)
}

/// One entry per distinct sha, plus a display name preferring the
/// `heads/` spelling over the `remotes/` alias.
fn record_tip(
    tips: &mut Vec<String>,
    names_by_sha: &mut HashMap<String, String>,
    sha: String,
    refname: String,
) {
    if !tips.iter().any(|t| t == &sha) {
        tips.push(sha.clone());
    }
    let display = refname
        .trim_start_matches("refs/heads/")
        .trim_start_matches("refs/remotes/");
    names_by_sha
        .entry(sha)
        .or_insert_with(|| display.to_string());
}

fn is_own_ref(branch: Option<&str>, refname: &str) -> bool {
    match branch {
        Some(b) => refname == format!("refs/heads/{b}") || refname == format!("refs/remotes/{b}"),
        None => false,
    }
}

/// Shared-ancestor frontier of HEAD across all tips -- candidate fork
/// points from one merge-bases query. Bases that are proper ancestors
/// of another base are dropped so the deepest fork point wins.
fn boundary_bases_in(
    repo: &git2::Repository,
    head: &str,
    tips: &[String],
) -> Result<Vec<String>, String> {
    let mut set = Vec::with_capacity(tips.len() + 1);
    for rev in std::iter::once(head).chain(tips.iter().map(String::as_str)) {
        set.push(commit_oid(repo, rev)?);
    }
    let bases = merge_base_shas(repo, &set);
    Ok(drop_ancestor_bases(repo.workdir(), bases))
}

/// Common-ancestor oids of every participant, hex-encoded; empty when
/// they share no history.
fn merge_base_shas(repo: &git2::Repository, set: &[git2::Oid]) -> Vec<String> {
    match repo.merge_bases_many(set) {
        Ok(arr) => arr.iter().map(|o| o.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Remove every base that is a proper ancestor of another base: the
/// deepest fork point wins.
fn drop_ancestor_bases(dir: Option<&Path>, mut bases: Vec<String>) -> Vec<String> {
    let stale: Vec<usize> = (0..bases.len())
        .filter(|&i| (0..bases.len()).any(|j| i != j && is_ancestor_in(dir, &bases[i], &bases[j])))
        .collect();
    for i in stale.into_iter().rev() {
        bases.remove(i);
    }
    bases
}

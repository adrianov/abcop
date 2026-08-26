//! Fork-point detection: where the current branch last shared history
//! with a sibling. One `rev-list --boundary` pass replaces a
//! merge-base subprocess per branch, so detection cost stays flat no
//! matter how many branches the repository carries.

use std::collections::HashMap;
use std::path::Path;

use crate::git_changes::{current_branch_in, git_in, is_ancestor_in};

/// Newest commit shared by HEAD and any sibling branch tip -- the
/// parent/fork point. One `rev-list --boundary` pass replaces a
/// merge-base subprocess per branch, so detection cost stays flat no
/// matter how many branches the repository carries.
pub(super) fn fork_point_in(dir: Option<&Path>) -> Result<Option<(String, String)>, String> {
    let head = head_sha_in(dir)?;
    let branch = current_branch_in(dir);
    let Some((tips, names)) = sibling_tips_in(dir, &head, branch.as_deref())? else {
        return Ok(None);
    };

    let bases = boundary_bases_in(dir, &tips)?;
    let best = newest_by_commit_time_in(dir, &bases);
    Ok(best.map(|b| fork_label(&b, &names)))
}

fn head_sha_in(dir: Option<&Path>) -> Result<String, String> {
    Ok(git_in(dir, &["rev-parse", "HEAD"])?.trim().to_string())
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

/// Parse `for-each-ref` output into distinct sibling tips (own branch
/// excluded, one entry per sha) plus a display-name per sha, preferring
/// the `heads/` spelling over the `remotes/` alias. `None` when no other
/// branch exists.
fn sibling_tips_in(
    dir: Option<&Path>,
    head: &str,
    branch: Option<&str>,
) -> Result<Option<(Vec<String>, HashMap<String, String>)>, String> {
    let listing = git_in(
        dir,
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;

    let mut tips: Vec<String> = Vec::new();
    let mut names_by_sha: HashMap<String, String> = HashMap::new();
    for (sha, refname) in listing.lines().filter_map(parse_tip_line) {
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

/// `(sha, refname)` from one `%H %(refname)` output line.
fn parse_tip_line(line: &str) -> Option<(String, String)> {
    let (sha, refname) = line.split_once(' ')?;
    Some((sha.to_string(), refname.to_string()))
}

fn is_own_ref(branch: Option<&str>, refname: &str) -> bool {
    match branch {
        Some(b) => refname == format!("refs/heads/{b}") || refname == format!("refs/remotes/{b}"),
        None => false,
    }
}

/// Boundary commits of `HEAD ^tips` are exactly the shared-ancestor
/// frontier -- candidate fork points, collected in one pass. Bases that
/// are proper ancestors of another base are dropped so the deepest fork
/// point wins.
fn boundary_bases_in(dir: Option<&Path>, tips: &[String]) -> Result<Vec<String>, String> {
    let mut args: Vec<&str> = vec!["rev-list", "--boundary", "HEAD"];
    let negated = negations(tips);
    args.extend(negated.iter().map(String::as_str));
    let bases = boundary_shas(&git_in(dir, &args)?);
    Ok(drop_ancestor_bases(dir, bases))
}

fn negations(tips: &[String]) -> Vec<String> {
    tips.iter().map(|t| format!("^{t}")).collect()
}

/// Boundary-commit shas (`-`-prefixed lines) from `rev-list` output.
fn boundary_shas(out: &str) -> Vec<String> {
    let mut bases: Vec<String> = out
        .lines()
        .filter_map(|l| l.strip_prefix('-'))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    bases.sort();
    bases.dedup();
    bases
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

/// The boundary base with the newest committer date.
fn newest_by_commit_time_in(dir: Option<&Path>, bases: &[String]) -> Option<String> {
    bases
        .iter()
        .max_by_key(|b| {
            git_in(dir, &["show", "-s", "--format=%ct", b])
                .ok()
                .and_then(|s| s.trim().parse::<i64>().ok())
                .unwrap_or(0)
        })
        .cloned()
}

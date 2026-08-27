use std::time::UNIX_EPOCH;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = gix::discover(&path)?; // replaces: git rev-parse --show-toplevel

    println!("{}", probe_summary(&repo)?);
    Ok(())
}

/// Default-branch OID, its merge-base with HEAD, and the 36-hour reflog
/// OID, rendered as the single report line.
fn probe_summary(repo: &gix::Repository) -> Result<String, Box<dyn std::error::Error>> {
    let base = default_branch(repo)?; // replaces: `git rev-parse origin/main`
    let head = repo.rev_parse_single("HEAD")?;
    let mb = repo.merge_base(head.detach(), base.detach())?; // replaces git merge-base

    // reflog-date resolution for base@{36.hours.ago}
    let at_date = reflog_oid_36h_ago(repo)?;

    Ok(format!(
        "base={base} merge_base={mb} reflog_at_36h={at_date:?}"
    ))
}

/// First configured candidate that resolves, standing in for the
/// `origin/main || origin/master || main || master` shell fallback.
fn default_branch(repo: &gix::Repository) -> Result<gix::Id<'_>, &'static str> {
    ["origin/main", "origin/master", "main", "master"]
        .into_iter()
        .find_map(|r| repo.rev_parse_single(r).ok())
        .ok_or("no default branch found")
}

/// OID of `refs/remotes/origin/main` as of ~36 hours ago, resolved through
/// the reference's reflog timestamps (stands in for `base@{36.hours.ago}`).
fn reflog_oid_36h_ago(
    repo: &gix::Repository,
) -> Result<Option<gix::oid::ObjectId>, Box<dyn std::error::Error>> {
    let cutoff = cutoff_secs()?;
    let reference = repo.find_reference("refs/remotes/origin/main")?;
    let mut iter = reference.log_iter();
    let Some(log) = iter.all()? else {
        return Ok(None);
    };
    let mut at_date = None;
    for entry in log {
        let e = entry?;
        if reflog_time_secs(&e.signature.time) >= cutoff {
            at_date = Some(e.new_oid);
        }
    }
    Ok(at_date)
}

/// Unix timestamp for "now minus 36 hours".
fn cutoff_secs() -> Result<u64, Box<dyn std::error::Error>> {
    Ok(std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        - 36 * 3600)
}

/// Leading unix timestamp of a reflog signature-time value (`&str` or
/// `String`); 0 when absent or unparseable, mirroring the previous
/// parse-or-zero behavior.
fn reflog_time_secs(time: impl AsRef<str>) -> u64 {
    time.as_ref()
        .split_whitespace()
        .next()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0)
}

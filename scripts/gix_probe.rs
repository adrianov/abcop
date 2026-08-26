use std::time::{Duration, UNIX_EPOCH};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = gix::discover(&path)?; // replaces: git rev-parse --show-toplevel

    let base = ["origin/main", "origin/master", "main", "master"]
        .into_iter()
        .find_map(|r| repo.rev_parse_single(r).ok())
        .ok_or("no default branch found")?;

    let head = repo.rev_parse_single("HEAD")?;
    let mb = repo.merge_base(head.detach(), base.detach())?; // replaces git merge-base

    // reflog-date resolution for base@{36.hours.ago}
    let cutoff =
        std::time::SystemTime::now().duration_since(UNIX_EPOCH)? - Duration::from_secs(36 * 3600);
    let reference = repo.find_reference("refs/remotes/origin/main")?;
    let mut at_date = None;
    let mut iter = reference.log_iter();
    let log = iter.all()?;
    if let Some(log) = log {
        for entry in log {
            let e = entry?;
            let ts: u64 = e
                .signature
                .time
                .split_whitespace()
                .next()
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);
            if ts >= cutoff.as_secs() {
                at_date = Some(e.new_oid);
            }
        }
    }

    println!("base={base} merge_base={mb} reflog_at_36h={at_date:?}");
    Ok(())
}

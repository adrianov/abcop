//! Parallel directory traversal: explicit-file bypass, gitignore-aware
//! parallel walking with default-skip pruning, and the deterministic
//! breadth-first sort that keeps downstream results stable.

use crate::paths::is_code_path;
use crate::skip::skipped_by_default;

/// A discovered file plus its depth below the walked root.
#[derive(Debug)]
struct Found {
    path: std::path::PathBuf,
    depth: usize,
}

/// Walk directory roots breadth-first and return code files ordered by
/// depth (shallowest first), then by parent directory, extension and file
/// name. Explicit file arguments come first at depth 0.
///
/// The parallel walk itself is unordered; the deterministic order is
/// produced by the final multi-key sort over the recorded depths. Entries
/// below a root are filtered through `skipped_by_default` and the walker's
/// ignore rules -- unless `everything` is set (`--everything`), which lifts
/// every filter: gitignore, hidden files, vendored/generated pruning.
pub(crate) fn collect_files(paths: &[String], everything: bool) -> Vec<std::path::PathBuf> {
    let (explicit, roots) = classify_targets(paths);
    let mut all = explicit;
    all.extend(walk_roots(&roots, everything));
    breadth_first_order(all)
}

/// Split CLI targets: existing files are scanned directly (depth 0),
/// everything else that exists becomes a walked root; missing paths drop.
fn classify_targets(paths: &[String]) -> (Vec<Found>, Vec<std::path::PathBuf>) {
    let mut explicit = Vec::new();
    let mut roots = Vec::new();
    for raw in paths {
        let p = std::path::Path::new(raw);
        if p.is_file() {
            explicit.push(Found {
                path: p.to_path_buf(),
                depth: 0,
            });
        } else if p.exists() {
            roots.push(p.to_path_buf());
        }
    }
    (explicit, roots)
}

/// Walk every root on one shared parallel visitor and return the discovery.
fn walk_roots(roots: &[std::path::PathBuf], everything: bool) -> Vec<Found> {
    if roots.is_empty() {
        return Vec::new();
    }
    let discovered = std::sync::Mutex::new(Vec::new());
    let mut builder = ignore::WalkBuilder::new(&roots[0]);
    for r in &roots[1..] {
        builder.add(r);
    }
    builder.threads(std::thread::available_parallelism().map_or(1, |n| n.get()));
    if everything {
        // `--everything`: no ignore files, no hidden skipping --
        // literally every code file below the target.
        lift_all_filters(&mut builder);
    }
    let mut collector = CollectorBuilder {
        sink: &discovered,
        roots,
        no_skip: everything,
    };
    builder.build_parallel().visit(&mut collector);
    discovered.into_inner().unwrap_or_default()
}

/// `--everything` filter lift: gitignore (repo, global, exclude), hidden
/// entries, parent-dir lookups -- nothing below the target stays hidden.
fn lift_all_filters(builder: &mut ignore::WalkBuilder) {
    builder
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .require_git(false);
}

/// Deterministic report order from unordered parallel discovery:
/// shallowest first, then parent directory, extension, and file name.
fn breadth_first_order(mut all: Vec<Found>) -> Vec<std::path::PathBuf> {
    all.sort_by(found_order);
    all.dedup_by(|a, b| a.path == b.path);
    all.into_iter().map(|f| f.path).collect()
}

/// Multi-key comparator behind [`breadth_first_order`].
fn found_order(a: &Found, b: &Found) -> std::cmp::Ordering {
    a.depth
        .cmp(&b.depth)
        .then_with(|| a.parent().cmp(&b.parent()))
        .then_with(|| a.ext_key().cmp(&b.ext_key()))
        .then_with(|| a.name_key().cmp(&b.name_key()))
}

trait PathKey {
    fn parent(&self) -> String;
    fn ext_key(&self) -> String;
    fn name_key(&self) -> String;
}

impl PathKey for Found {
    fn parent(&self) -> String {
        self.path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }
    fn ext_key(&self) -> String {
        self.path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }
    fn name_key(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }
}

/// Shared-sink parallel visitor: one per worker thread.
struct CollectorBuilder<'s> {
    sink: &'s std::sync::Mutex<Vec<Found>>,
    roots: &'s [std::path::PathBuf],
    /// `--everything`: bypass default-skip pruning entirely.
    no_skip: bool,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for CollectorBuilder<'s> {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(Collector {
            sink: self.sink,
            roots: self.roots,
            no_skip: self.no_skip,
        })
    }
}

struct Collector<'s> {
    sink: &'s std::sync::Mutex<Vec<Found>>,
    roots: &'s [std::path::PathBuf],
    no_skip: bool,
}

impl ignore::ParallelVisitor for Collector<'_> {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        let Ok(entry) = entry else {
            return ignore::WalkState::Continue;
        };
        if !self.no_skip
            && let Some(state) = prune_state(&entry, self.roots)
        {
            return state;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && is_code_path(entry.path())
            && let Ok(mut sink) = self.sink.lock()
        {
            sink.push(Found {
                depth: entry.depth(),
                path: entry.into_path(),
            });
        }
        ignore::WalkState::Continue
    }
}

/// WalkState when this entry should prune out of the default walk: whole
/// generated/vendor/test subtrees are skipped during traversal itself,
/// individual files simply drop through.
fn prune_state(
    entry: &ignore::DirEntry,
    roots: &[std::path::PathBuf],
) -> Option<ignore::WalkState> {
    if entry.depth() == 0 || !skipped_by_default(entry.path(), roots) {
        return None;
    }
    Some(if entry.file_type().is_some_and(|t| t.is_dir()) {
        ignore::WalkState::Skip
    } else {
        ignore::WalkState::Continue
    })
}

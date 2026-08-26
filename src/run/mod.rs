//! One configured scan run: scope resolution, file collection, per-file
//! analysis fan-out and reporting. Scope decisions themselves live in
//! [`crate::scan_scope`].

use std::process::ExitCode;

use rayon::prelude::*;

use crate::output::FileResult;
use crate::walker::collect_files;
use crate::{git_changes, pipeline, scan_scope};

/// A single invocation's scan configuration. Built from the parsed CLI;
/// `execute` owns the whole pipeline from scope resolution to exit code.
pub(crate) struct ScanRun<'a> {
    paths: &'a [String],
    only: Option<&'a str>,
    max_abc: f64,
    format: &'a str,
    mr: bool,
    full: bool,
    everything: bool,
    no_cache: bool,
}

impl<'a> From<&'a super::Cli> for ScanRun<'a> {
    fn from(cli: &'a super::Cli) -> Self {
        Self {
            paths: &cli.paths,
            only: cli.only.as_deref(),
            max_abc: cli.max_abc,
            format: &cli.format,
            mr: cli.mr,
            full: cli.full,
            everything: cli.everything,
            no_cache: cli.no_cache,
        }
    }
}

impl ScanRun<'_> {
    /// Execute the configured scan and map the outcome to the process exit
    /// code: 0 clean, 1 diagnostics reported.
    pub(crate) fn execute(&self) -> ExitCode {
        let explicit_paths = !self.paths.is_empty();
        let changeset =
            match scan_scope::resolve(self.mr, explicit_paths, self.full, self.everything) {
                Ok(v) => v,
                Err(e) => return scan_scope::error(e),
            };
        let cache = self.open_cache();
        let start = std::time::Instant::now();
        let results = self.scan(explicit_paths, changeset.as_ref(), cache.as_ref());
        self.render(&results, start.elapsed());
        exit_code(&results)
    }

    /// Collect targets and run the per-file pipeline over them in walker
    /// order.
    fn scan(
        &self,
        explicit_paths: bool,
        changeset: Option<&git_changes::Changeset>,
        cache: Option<&crate::cache::Cache>,
    ) -> Vec<FileResult> {
        // MR/changed scope picks its own files; otherwise walk the targets.
        // Whole-tree modes and the no-repo fallback both start at cwd.
        let files = collect_targets(self, explicit_paths, changeset);
        scan_scope::note_if_empty(changeset, &files);

        // par_iter keeps the walker's (BFS + extension/name) order intact
        files
            .par_iter()
            .map(|p| pipeline::analyze_one(p, self.only, self.max_abc, changeset, cache))
            .collect()
    }

    /// Open the on-disk result cache unless disabled, pruning stale entries.
    fn open_cache(&self) -> Option<crate::cache::Cache> {
        let cache = if self.no_cache {
            None
        } else {
            crate::cache::Cache::open(false)
        };
        if let Some(cache) = cache.as_ref() {
            cache.prune();
        }
        cache
    }

    fn render(&self, results: &[FileResult], elapsed: std::time::Duration) {
        match self.format {
            "json" => {
                crate::output::print_json(&results.len(), results, elapsed);
            }
            _ => crate::output::print_text(results, self.max_abc, elapsed),
        }
    }
}

/// Files this run analyses: an MR/changed scope picks its own set;
/// otherwise walk the named targets, or cwd for whole-tree modes and
/// the no-repository fallback.
fn collect_targets(
    run: &ScanRun<'_>,
    explicit_paths: bool,
    changeset: Option<&git_changes::Changeset>,
) -> Vec<std::path::PathBuf> {
    match changeset {
        Some(cs) => cs
            .code_files()
            .into_iter()
            .filter(|p| !crate::modulesize::is_route_table(p))
            .filter(|p| !crate::modulesize::is_third_party(p))
            .collect(),
        None if explicit_paths => collect_files(run.paths, run.everything),
        None => collect_files(&[String::from(".")], run.everything),
    }
}

fn exit_code(results: &[FileResult]) -> ExitCode {
    let clean = results.iter().all(|r| {
        r.abc.is_empty()
            && r.used_once.is_empty()
            && r.never_used.is_empty()
            && r.oversize.is_none()
    });
    if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

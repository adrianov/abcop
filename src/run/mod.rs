//! One configured scan run: scope resolution, file collection, per-file
//! analysis fan-out and reporting.

use std::process::ExitCode;

use rayon::prelude::*;

use crate::output::FileResult;
use crate::{git_changes, pipeline};
pub(crate) use crate::walker::collect_files;

/// A single invocation's scan configuration. Built from the parsed CLI;
/// `execute` owns the whole pipeline from scope resolution to exit code.
pub(crate) struct ScanRun<'a> {
    paths: &'a [String],
    only: Option<&'a str>,
    max_abc: f64,
    format: &'a str,
    changed: bool,
    mr: bool,
    full: bool,
    everything: bool,
    base: Option<&'a str>,
    no_cache: bool,
}

impl<'a> From<&'a super::Cli> for ScanRun<'a> {
    fn from(cli: &'a super::Cli) -> Self {
        Self {
            paths: &cli.paths,
            only: cli.only.as_deref(),
            max_abc: cli.max_abc,
            format: &cli.format,
            changed: cli.changed,
            mr: cli.mr,
            full: cli.full,
            everything: cli.everything,
            base: cli.base.as_deref(),
            no_cache: cli.no_cache,
        }
    }
}

impl ScanRun<'_> {
    /// Execute the configured scan and map the outcome to the process exit
    /// code: 0 clean, 1 diagnostics reported.
    pub(crate) fn execute(&self) -> ExitCode {
        let explicit_paths = !self.paths.is_empty();
        let changeset = match self.resolve_scope(explicit_paths) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        };
        let cache =
            if self.no_cache { None } else { crate::cache::Cache::open(false) };
        if let Some(cache) = cache.as_ref() {
            cache.prune();
        }
        let start = std::time::Instant::now();

        // MR/changed scope picks its own files; otherwise walk the targets.
        let files: Vec<std::path::PathBuf> = match changeset.as_ref() {
            Some(cs) => cs.code_files(),
            None => collect_files(self.paths, self.everything),
        };
        // par_iter keeps the walker's (BFS + extension/name) order intact
        let results: Vec<FileResult> = files
            .par_iter()
            .map(|p| {
                pipeline::analyze_one(
                    p,
                    self.only,
                    self.max_abc,
                    changeset.as_ref(),
                    cache.as_ref(),
                )
            })
            .collect();

        self.render(&results, start.elapsed());
        exit_code(&results)
    }

    /// Resolve which git-scope applies. Bare invocations default to the MR
    /// scope; outside a repository -- or with no detectable base -- they
    /// fall back to a full-tree scan rather than failing.
    fn resolve_scope(
        &self,
        explicit_paths: bool,
    ) -> Result<Option<git_changes::Changeset>, String> {
        if self.changed {
            let base = self.base.unwrap_or("HEAD");
            return git_changes::Changeset::load(base).map(Some);
        }
        if self.mr {
            return load_mr_scope().map(Some);
        }
        if !explicit_paths && !self.full && !self.everything {
            // Default mode: review what changed, like CI would.
            return match load_mr_scope() {
                Ok(cs) => Ok(Some(cs)),
                Err(e) => {
                    eprintln!("note: no MR scope ({e}); scanning the full tree");
                    Ok(None)
                }
            };
        }
        Ok(None)
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

fn load_mr_scope() -> Result<git_changes::Changeset, String> {
    let (base, label) = git_changes::mr_base()?;
    eprintln!("--mr scope: {label} (base {base})");
    git_changes::Changeset::load(&base)
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

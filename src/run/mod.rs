//! One configured scan run: scope resolution, file collection, per-file
//! analysis fan-out and reporting. Scope decisions themselves live in
//! [`crate::scan_scope`].

use std::process::ExitCode;
use std::sync::Mutex;

use rayon::prelude::*;

use crate::output::{FileResult, JsonStream, RunStats};
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
    uncommitted: bool,
    full: bool,
    everything: bool,
    no_cache: bool,
    sort_by_score: bool,
}

impl<'a> From<&'a super::Cli> for ScanRun<'a> {
    fn from(cli: &'a super::Cli) -> Self {
        Self {
            paths: &cli.paths,
            only: cli.only.as_deref(),
            max_abc: cli.max_abc,
            format: &cli.format,
            mr: cli.mr,
            uncommitted: cli.uncommitted,
            full: cli.full,
            everything: cli.everything,
            no_cache: cli.no_cache,
            sort_by_score: cli.sort_by_score,
        }
    }
}

struct Prepared {
    changeset: Option<git_changes::Changeset>,
    cache: Option<crate::cache::Cache>,
    files: Vec<std::path::PathBuf>,
}

impl ScanRun<'_> {
    /// Execute the configured scan and map the outcome to the process exit
    /// code: 0 clean, 1 diagnostics reported.
    pub(crate) fn execute(&self) -> ExitCode {
        let prepared = match self.prepare() {
            Ok(p) => p,
            Err(code) => return code,
        };
        let start = std::time::Instant::now();
        // --sort-by-score needs every result before emit; otherwise each
        // file prints as soon as its analysis finishes (text and JSON).
        if self.sort_by_score {
            self.run_buffered(&prepared, start)
        } else {
            self.run_stream(&prepared, start)
        }
    }

    fn prepare(&self) -> Result<Prepared, ExitCode> {
        let explicit_paths = !self.paths.is_empty();
        let changeset = scan_scope::resolve(
            self.mr,
            self.uncommitted,
            explicit_paths,
            self.full,
            self.everything,
        )
        .map_err(scan_scope::error)?;
        let cache = self.open_cache();
        let files = collect_targets(self, explicit_paths, changeset.as_ref());
        scan_scope::note_if_empty(changeset.as_ref(), &files);
        Ok(Prepared {
            changeset,
            cache,
            files,
        })
    }

    fn run_buffered(&self, prepared: &Prepared, start: std::time::Instant) -> ExitCode {
        let results = self.scan_all(
            &prepared.files,
            prepared.changeset.as_ref(),
            prepared.cache.as_ref(),
        );
        match self.format {
            "json" => crate::output::print_json(&results.len(), &results, start.elapsed()),
            _ => crate::output::print_text_sorted(&results, self.max_abc, start.elapsed()),
        }
        exit_code(&results)
    }

    fn run_stream(&self, prepared: &Prepared, start: std::time::Instant) -> ExitCode {
        match self.format {
            "json" => self.stream_json(prepared, start),
            _ => self.stream_text(prepared, start),
        }
    }

    fn stream_text(&self, prepared: &Prepared, start: std::time::Instant) -> ExitCode {
        let sink = Mutex::new(RunStats::default());
        self.for_each_file(prepared, |r| {
            let mut sink = sink.lock().unwrap();
            crate::output::print_file_text(r, self.max_abc);
            sink.add(r);
        });
        let stats = sink.into_inner().unwrap();
        crate::output::print_summary(&stats, start.elapsed());
        exit_from_stats(&stats)
    }

    fn stream_json(&self, prepared: &Prepared, start: std::time::Instant) -> ExitCode {
        let sink = Mutex::new((JsonStream::begin(), RunStats::default()));
        self.for_each_file(prepared, |r| {
            let mut sink = sink.lock().unwrap();
            sink.0.write_file(r);
            sink.1.add(r);
        });
        let (stream, stats) = sink.into_inner().unwrap();
        stream.finish(&stats, start.elapsed());
        exit_from_stats(&stats)
    }

    /// Analyse every file, collecting results in walker order.
    fn scan_all(
        &self,
        files: &[std::path::PathBuf],
        changeset: Option<&git_changes::Changeset>,
        cache: Option<&crate::cache::Cache>,
    ) -> Vec<FileResult> {
        files
            .par_iter()
            .map(|p| pipeline::analyze_one(p, self.only, self.max_abc, changeset, cache))
            .collect()
    }

    /// Run analysis in parallel; `on_file` is called once per finished
    /// file (caller serializes shared output state).
    fn for_each_file(&self, prepared: &Prepared, on_file: impl Fn(&FileResult) + Sync) {
        prepared.files.par_iter().for_each(|p| {
            let r = pipeline::analyze_one(
                p,
                self.only,
                self.max_abc,
                prepared.changeset.as_ref(),
                prepared.cache.as_ref(),
            );
            on_file(&r);
        });
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
    if results.iter().all(|r| r.is_clean()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn exit_from_stats(stats: &RunStats) -> ExitCode {
    if stats.dirty {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

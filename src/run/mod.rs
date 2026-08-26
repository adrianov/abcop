//! One configured scan run: scope resolution, file collection, per-file
//! analysis fan-out and reporting.

use std::process::ExitCode;

use rayon::prelude::*;

use crate::output::FileResult;
pub(crate) use crate::walker::collect_files;
use crate::{git_changes, pipeline};

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
            Err(e) => return scope_error(e),
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
        let files = self.collect_targets(explicit_paths, changeset);
        self.note_if_scope_empty(changeset, &files);

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

    /// Files this run analyses: an MR/changed scope picks its own set;
    /// otherwise walk the named targets, or cwd for whole-tree modes and
    /// the no-repository fallback.
    fn collect_targets(
        &self,
        explicit_paths: bool,
        changeset: Option<&git_changes::Changeset>,
    ) -> Vec<std::path::PathBuf> {
        match changeset {
            Some(cs) => cs
                .code_files()
                .into_iter()
                .filter(|p| !crate::modulesize::is_route_table(p))
                .collect(),
            None if explicit_paths => collect_files(self.paths, self.everything),
            None => collect_files(&[String::from(".")], self.everything),
        }
    }

    /// Hint when a scoped run matched nothing: likely a stale scope.
    fn note_if_scope_empty(
        &self,
        changeset: Option<&git_changes::Changeset>,
        files: &[std::path::PathBuf],
    ) {
        if changeset.is_some() && files.is_empty() {
            eprintln!("note: nothing changed in scope; try --full to scan the whole tree");
        }
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
            // Default mode: review the working state -- uncommitted work
            // against HEAD plus everything the branch already changed vs
            // its base -- the union CI would gate on.
            let head = git_changes::Changeset::load("HEAD");
            return match load_mr_scope() {
                Ok(mr) => Ok(Some(match head {
                    Ok(h) => merge_scopes(mr, h),
                    Err(_) => mr,
                })),
                Err(e) => match head {
                    Ok(h) if !h.files.is_empty() => Ok(Some(h)),
                    _ => {
                        eprintln!("note: no MR scope ({e}); scanning the full tree");
                        Ok(None)
                    }
                },
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

/// Union of two change scopes over the same repository: a file present in
/// either counts, and per-line ranges widen (`All` dominates).
fn merge_scopes(
    mut base: git_changes::Changeset,
    extra: git_changes::Changeset,
) -> git_changes::Changeset {
    use std::collections::btree_map::Entry;
    for (path, lines) in extra.files {
        match base.files.entry(path) {
            Entry::Vacant(v) => {
                v.insert(lines);
            }
            Entry::Occupied(mut o) => {
                let merged = match (o.get(), lines) {
                    (git_changes::Lines::All, _) | (_, git_changes::Lines::All) => {
                        git_changes::Lines::All
                    }
                    (git_changes::Lines::Ranges(a), git_changes::Lines::Ranges(b)) => {
                        let mut u = a.clone();
                        u.extend(b);
                        git_changes::Lines::Ranges(u)
                    }
                };
                o.insert(merged);
            }
        }
    }
    base
}

fn load_mr_scope() -> Result<git_changes::Changeset, String> {
    let (base, label) = git_changes::mr_base()?;
    eprintln!("--mr scope: {label} (base {base})");
    git_changes::Changeset::load(&base)
}

/// Scope resolution failure: report it and use the scope-error exit code.
fn scope_error(e: String) -> ExitCode {
    eprintln!("{e}");
    ExitCode::from(2)
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

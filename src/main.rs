//! abcop — fast multi-language ABC-size and used-once-variable linter.

mod abc;
mod abc_count;
mod directives;
mod model;
mod never_used;
mod modulesize;
mod output;
mod pipeline;
mod clike;
mod dump;
mod cache;
mod git_changes;
mod paths;
mod rustlang;
mod skip;
mod walker;
mod used_once;

use std::process::ExitCode;

use clap::Parser as ClapParser;
use rayon::prelude::*;
pub use crate::model::build;
use output::FileResult;
use pipeline::analyze_one;
use walker::collect_files;

#[derive(ClapParser)]
#[command(name = "abcop", version, about = "Fast multi-language ABC-size + used-once-variable linter")]
struct Cli {
    /// Files or directories to analyse; omitted means current-MR scope
    paths: Vec<String>,
    /// Output format
    #[arg(long, value_parser = ["text", "json"], default_value = "text")]
    format: String,
    /// Maximum ABC score before reporting
    #[arg(long, default_value_t = 17.0)]
    max_abc: f64,
    /// Run only one of the checks
    #[arg(long, value_parser = ["abc", "used-once", "never-used"])]
    only: Option<String>,
    /// Only report on functions whose lines are changed in git
    #[arg(long)]
    changed: bool,
    /// Scan the last MR: changes since branching from master/main, or the
    /// last 36 hours when committing directly onto it
    #[arg(long, conflicts_with = "changed")]
    mr: bool,
    /// Scan the whole production tree instead of the current MR (default
    /// skips for vendored/generated/test trees stay active)
    #[arg(long, conflicts_with_all = ["changed", "mr", "everything"])]
    full: bool,
    /// Scan literally everything below the target: no gitignore, no hidden
    /// skipping, no vendored/generated/test pruning
    #[arg(long, conflicts_with_all = ["changed", "mr", "full", "base"])]
    everything: bool,
    /// Git base ref for --changed (default HEAD)
    #[arg(long, requires = "changed")]
    base: Option<String>,
    /// Disable the on-disk result cache
    #[arg(long)]
    no_cache: bool,
    /// Debug: dump the syntax tree of a single file
    #[arg(long, hide = true)]
    dump_tree: bool,
}

fn main() -> ExitCode {
    // die quietly on SIGPIPE instead of panicking when piped into head/less
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let mut cli = Cli::parse();

    if cli.dump_tree {
        return run_dump_tree(&cli.paths);
    }
    // Whole-tree modes need a target; bare `--full`/`--everything` scan cwd.
    if cli.paths.is_empty() && (cli.full || cli.everything) {
        cli.paths.push(String::from("."));
    }
    run_scan(&cli)
}

fn run_dump_tree(paths: &[String]) -> ExitCode {
    let Some(path) = paths.first() else {
        eprintln!("--dump-tree requires a file path");
        return ExitCode::from(2);
    };
    match dump::dump_tree(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

fn run_scan(cli: &Cli) -> ExitCode {
    let explicit_paths = !cli.paths.is_empty();
    let changeset = match resolve_scope(cli, explicit_paths) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let cache = if cli.no_cache { None } else { crate::cache::Cache::open(false) };
    if let Some(cache) = cache.as_ref() {
        cache.prune();
    }
    let start = std::time::Instant::now();
    let results = scan_paths(
        &cli.paths,
        cli.only.as_deref(),
        cli.max_abc,
        changeset.as_ref(),
        cache.as_ref(),
        cli.everything,
    );
    render(&results, &cli.format, cli.max_abc, start.elapsed());
    exit_code(&results)
}

/// Resolve which git-scope applies. Bare invocations default to the MR
/// scope; outside a repository -- or with no detectable base -- they fall
/// back to a full-tree scan rather than failing.
fn resolve_scope(
    cli: &Cli,
    explicit_paths: bool,
) -> Result<Option<git_changes::Changeset>, String> {
    if cli.changed {
        let base = cli.base.as_deref().unwrap_or("HEAD");
        return git_changes::Changeset::load(base).map(Some);
    }
    if cli.mr {
        return load_mr_scope().map(Some);
    }
    if !explicit_paths && !cli.full && !cli.everything {
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

fn load_mr_scope() -> Result<git_changes::Changeset, String> {
    let (base, label) = git_changes::mr_base()?;
    eprintln!("--mr scope: {label} (base {base})");
    git_changes::Changeset::load(&base)
}

#[allow(clippy::too_many_arguments)]
fn scan_paths(
    paths: &[String],
    only: Option<&str>,
    max: f64,
    changeset: Option<&git_changes::Changeset>,
    cache: Option<&crate::cache::Cache>,
    everything: bool,
) -> Vec<FileResult> {
    let files: Vec<std::path::PathBuf> = match changeset {
        Some(cs) => cs.code_files(),
        None => collect_files(paths, everything),
    };
    // par_iter keeps the caller's (BFS + extension/name) order intact
    let results: Vec<FileResult> = files
        .par_iter()
        .map(|p| analyze_one(p, only, max, changeset, cache))
        .collect();    results
}

fn render(
    results: &[FileResult],
    format: &str,
    max: f64,
    elapsed: std::time::Duration,
) {
    match format {
        "json" => output::print_json(&results.len(), results, elapsed),
        _ => output::print_text(results, max, elapsed),
    }
}

fn exit_code(results: &[FileResult]) -> ExitCode {
    let clean = results
        .iter()
        .all(|r| {
            r.abc.is_empty() && r.used_once.is_empty() && r.never_used.is_empty()
                && r.oversize.is_none()
        });
    if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}


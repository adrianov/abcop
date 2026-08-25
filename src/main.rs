//! abcop — fast multi-language ABC-size and used-once-variable linter.

mod abc;
mod abc_count;
mod directives;
mod model;
mod never_used;
mod modulesize;
mod output;
mod pipeline;
mod dump;
mod git_changes;
mod paths;
mod rustlang;
mod used_once;

use std::process::ExitCode;

use clap::Parser as ClapParser;
use rayon::prelude::*;
use serde::Serialize;

use crate::abc::AbcOffense;
pub use crate::model::build;
use paths::collect_files;
use pipeline::analyze_one;
use never_used::NeverUsedOffense;
use used_once::UsedOnceOffense;

#[derive(ClapParser)]
#[command(name = "abcop", about = "Fast multi-language ABC-size + used-once-variable linter")]
struct Cli {
    /// Files or directories to analyse
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
    /// Git base ref for --changed (default HEAD)
    #[arg(long)]
    base: Option<String>,
    /// Debug: dump the syntax tree of a single file
    #[arg(long, hide = true)]
    dump_tree: bool,
}

#[derive(Serialize)]
struct Diagnostic {
    file: String,
    line: usize,
    column: usize,
    severity: &'static str,
    rule: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector: Option<String>,
}

pub(crate) struct FileResult {
    pub path: String,
    pub abc: Vec<AbcOffense>,
    pub used_once: Vec<UsedOnceOffense>,
    pub never_used: Vec<NeverUsedOffense>,
    pub oversize: Option<usize>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.dump_tree {
        return run_dump_tree(&cli.paths);
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
    if cli.paths.is_empty() {
        eprintln!("no paths given");
        return ExitCode::from(2);
    }
    let changeset = if cli.changed {
        let base = cli.base.as_deref().unwrap_or("HEAD");
        match git_changes::Changeset::load(base) {
            Ok(cs) => Some(cs),
            Err(e) => {
                eprintln!("--changed: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };
    let results = scan_paths(&cli.paths, cli.only.as_deref(), cli.max_abc, changeset.as_ref());
    render(&results, &cli.format, cli.max_abc);
    exit_code(&results)
}

fn scan_paths(
    paths: &[String],
    only: Option<&str>,
    max: f64,
    changeset: Option<&git_changes::Changeset>,
) -> Vec<FileResult> {
    let files: Vec<std::path::PathBuf> = match changeset {
        Some(cs) => cs.code_files(),
        None => collect_files(paths),
    };
    let mut results: Vec<FileResult> = files
        .par_iter()
        .map(|p| analyze_one(p, only, max, changeset))
        .collect();
    results.sort_by(|x, y| x.path.cmp(&y.path));
    results
}

fn render(results: &[FileResult], format: &str, max: f64) {
    match format {
        "json" => output::print_json(&results.len(), results),
        _ => output::print_text(results, max),
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


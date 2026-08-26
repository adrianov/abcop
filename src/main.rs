//! abcop — fast multi-language ABC-size and used-once-variable linter.

mod abc;
mod abc_count;
mod cache;
mod clike;
mod directives;
mod dump;
mod git_changes;
mod model;
mod modulesize;
mod never_used;
mod output;
mod paths;
mod pipeline;
mod srcbuf;
mod run;
mod rustlang;
mod skip;
mod used_once;
mod walker;

use std::process::ExitCode;

use clap::Parser as ClapParser;

/// Parsed command line: the surface that builds a [`run::ScanRun`].
#[derive(ClapParser)]
#[command(name = "abcop", version, about = "Fast multi-language ABC-size + used-once-variable linter")]
pub(crate) struct Cli {
    /// Files or directories to analyse; omitted means current-MR scope
    pub(crate) paths: Vec<String>,
    /// Output format
    #[arg(long, value_parser = ["text", "json"], default_value = "text")]
    pub(crate) format: String,
    /// Maximum ABC score before reporting
    #[arg(long, default_value_t = 17.0)]
    pub(crate) max_abc: f64,
    /// Run only one of the checks
    #[arg(long, value_parser = ["abc", "used-once", "never-used"])]
    pub(crate) only: Option<String>,
    /// Only report on functions whose lines are changed in git
    #[arg(long)]
    pub(crate) changed: bool,
    /// Scan the last MR: changes since branching from master/main, or the
    /// last 36 hours when committing directly onto it
    #[arg(long, conflicts_with = "changed")]
    pub(crate) mr: bool,
    /// Scan the whole production tree instead of the current MR (default
    /// skips for vendored/generated/test trees stay active)
    #[arg(long, conflicts_with_all = ["changed", "mr", "everything"])]
    pub(crate) full: bool,
    /// Scan literally everything below the target: no gitignore, no hidden
    /// skipping, no vendored/generated/test pruning
    #[arg(long, conflicts_with_all = ["changed", "mr", "full", "base"])]
    pub(crate) everything: bool,
    /// Git base ref for --changed (default HEAD)
    #[arg(long, requires = "changed")]
    pub(crate) base: Option<String>,
    /// Disable the on-disk result cache
    #[arg(long)]
    pub(crate) no_cache: bool,
    /// Debug: dump the syntax tree of a single file
    #[arg(long, hide = true)]
    dump_tree: bool,
}

fn main() -> ExitCode {
    // die quietly on SIGPIPE instead of panicking when piped into head/less
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();

    if cli.dump_tree {
        return run_dump_tree(&cli.paths);
    }
    run::ScanRun::from(&cli).execute()
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

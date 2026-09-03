//! abcop — must-have ABC complexity gate for AI development.

mod abc;
mod cache;
mod clike;
mod csharp;
mod dart;
mod directives;
mod dump;
mod fork_point;
mod git_changes;
#[cfg(test)]
mod git_changes_tests;
mod golang;
mod inlinable;
mod javalang;
mod model;
mod modulesize;
mod mr_scope;
#[cfg(test)]
mod mr_scope_tests;
mod never_used;
mod output;
mod paths;
mod phplang;
mod pipeline;
mod pylang;
mod repo_state;
mod run;
mod rustlang;
mod scan_scope;
#[cfg(test)]
mod scan_scope_tests;
mod scope_model;
mod skip;
mod sollang;
mod srcbuf;
mod ziglang;
mod untracked_scan;

#[cfg(test)]
mod test_repo;
mod used_once;
mod walker;

use std::process::ExitCode;

use clap::Parser as ClapParser;

/// Parsed command line: the surface that builds a [`run::ScanRun`].
#[derive(ClapParser)]
#[command(
    name = "abcop",
    version,
    about = "Must-have ABC complexity gate for AI development. Ruby, Rust, Python, Go, JS/TS, C/C++, PHP, Java, C#, Swift, Zig, Dart, Solidity, ObjC"
)]
pub(crate) struct Cli {
    /// Files or directories to analyse; omitted means the auto-selected
    /// scope: uncommitted work when the tree is dirty, else the MR scope,
    /// else the full tree
    pub(crate) paths: Vec<String>,
    /// Output format
    #[arg(long, value_parser = ["text", "json", "jsonl"], default_value = "text")]
    pub(crate) format: String,
    /// Maximum ABC score before reporting
    #[arg(long, default_value_t = 17.0)]
    pub(crate) max_abc: f64,
    /// Maximum module ABC score before reporting
    #[arg(long, default_value_t = modulesize::MAX_ABC)]
    pub(crate) max_module_abc: f64,
    /// Run only one of the checks
    #[arg(long, value_parser = ["abc", "used-once", "never-used"])]
    pub(crate) only: Option<String>,
    /// Scan the last MR explicitly: changes since branching from
    /// master/main plus uncommitted work (the bare default prefers
    /// uncommitted work when the tree is dirty)
    #[arg(long)]
    pub(crate) mr: bool,
    /// Scan only uncommitted work: working-tree and index edits vs
    /// HEAD plus untracked files -- no branch/base diff (what the
    /// bare default picks automatically when such work exists)
    #[arg(long, conflicts_with_all = ["mr", "full", "everything"])]
    pub(crate) uncommitted: bool,
    /// Scan the whole production tree instead of the scoped run (default
    /// skips for vendored/generated/test trees stay active)
    #[arg(long, conflicts_with_all = ["mr", "everything"])]
    pub(crate) full: bool,
    /// Scan literally everything below the target: no gitignore, no hidden
    /// skipping, no vendored/generated/test pruning
    #[arg(long, conflicts_with_all = ["mr", "full"])]
    pub(crate) everything: bool,
    /// Disable the on-disk result cache
    #[arg(long)]
    pub(crate) no_cache: bool,
    /// Buffer all findings and print highest-score first
    #[arg(long)]
    pub(crate) sort_by_score: bool,
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

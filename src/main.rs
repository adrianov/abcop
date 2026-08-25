//! abcop — fast multi-language ABC-size and used-once-variable linter.

mod abc;
mod abc_count;
mod directives;
mod model;
mod modulesize;
mod output;
mod dump;
mod paths;
mod rustlang;
mod used_once;

use std::process::ExitCode;

use clap::Parser as ClapParser;
use rayon::prelude::*;
use serde::Serialize;

use crate::abc::AbcOffense;
pub use crate::model::build;
use paths::{collect_files, lang_for, parse_file_lang, Lang};
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
    #[arg(long, value_parser = ["abc", "used-once"])]
    only: Option<String>,
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
    let results = scan_paths(&cli.paths, cli.only.as_deref(), cli.max_abc);
    render(&results, &cli.format, cli.max_abc);
    exit_code(&results)
}

fn scan_paths(paths: &[String], only: Option<&str>, max: f64) -> Vec<FileResult> {
    let files = collect_files(paths);
    let mut results: Vec<FileResult> = files
        .par_iter()
        .map(|p| analyze_one(p, only, max))
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
        .all(|r| r.abc.is_empty() && r.used_once.is_empty() && r.oversize.is_none());
    if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn analyze_one(path: &std::path::PathBuf, only: Option<&str>, max: f64) -> FileResult {
    let blank = FileResult {
        path: path.display().to_string(),
        abc: Vec::new(),
        used_once: Vec::new(),
        oversize: None,
    };
    let Some((lang, src_bytes, tree)) = load(path) else {
        return blank;
    };
    let checks = Checks::new(only);
    let oversize =
        std::str::from_utf8(&src_bytes).ok().and_then(|t| modulesize::offense(t, &path.display().to_string()));
    match lang {
        Lang::Rust => {
            let fm = rustlang::build(src_bytes, tree);
            FileResult {
                path: blank.path,
                abc: if checks.want_abc {
                    rustlang::analyze(&fm, max)
                } else {
                    Vec::new()
                },
                used_once: if checks.want_used {
                    rustlang::used_once_offenses(&fm)
                } else {
                    Vec::new()
                },
                oversize,
            }
        }
        Lang::Ruby => {
            let text = String::from_utf8_lossy(&src_bytes).into_owned();
            let dirs = directives::parse(&text);
            let mut r = blank_with(path);
            if checks.want_abc {
                r.abc = ruby_abc(&src_bytes, lang, &dirs, max);
            }
            if checks.want_used {
                r.used_once = ruby_used(&src_bytes, lang, &dirs);
            }
            r
        }
    }
}


struct Checks {
    want_abc: bool,
    want_used: bool,
}

impl Checks {
    fn new(only: Option<&str>) -> Self {
        Self {
            want_abc: only.is_none_or(|o| o == "abc"),
            want_used: only.is_none_or(|o| o == "used-once"),
        }
    }
}

fn blank_with(path: &std::path::Path) -> FileResult {
    FileResult {
        path: path.display().to_string(),
        abc: Vec::new(),
        used_once: Vec::new(),
        oversize: None,
    }
}

fn reparsed(src_bytes: &[u8], lang: Lang) -> Option<crate::model::FileModel> {
    let tree = parse_file_lang(src_bytes, lang)?;
    Some(model::build(src_bytes.to_vec(), tree))
}

fn ruby_abc(
    src_bytes: &[u8],
    lang: Lang,
    dirs: &directives::Directives,
    max: f64,
) -> Vec<AbcOffense> {
    let Some(fm) = reparsed(src_bytes, lang) else { return Vec::new() };
    abc::analyze(&fm, max)
        .into_iter()
        .filter(|o| !dirs.suppresses_abc(o.line))
        .collect()
}

fn ruby_used(
    src_bytes: &[u8],
    lang: Lang,
    dirs: &directives::Directives,
) -> Vec<UsedOnceOffense> {
    let Some(fm) = reparsed(src_bytes, lang) else { return Vec::new() };
    used_once::analyze(&fm)
        .into_iter()
        .filter(|o| !dirs.suppresses_all(o.line))
        .collect()
}

fn load(path: &std::path::PathBuf) -> Option<(Lang, Vec<u8>, tree_sitter::Tree)> {
    let file = std::fs::File::open(path).ok()?;
    let cap = file.metadata().ok()?.len() as usize;
    let mut src_bytes = Vec::with_capacity(cap);
    {
        use std::io::Read;
        (&file).read_to_end(&mut src_bytes).ok()?;
    }
    let lang = lang_for(path);
    let tree = parse_file_lang(&src_bytes, lang)?;
    Some((lang, src_bytes, tree))
}

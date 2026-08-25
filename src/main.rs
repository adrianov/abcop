mod abc;
mod model;
mod used_once;

use std::fs;
use std::process::ExitCode;

use clap::Parser as ClapParser;
use rayon::prelude::*;
use serde::Serialize;
use tree_sitter::Parser;

use crate::abc::AbcOffense;
use crate::model::build;
use crate::used_once::UsedOnceOffense;

#[derive(ClapParser)]
#[command(name = "abcop", about = "Fast Ruby ABC-size + used-once-variable linter (tree-sitter based)")]
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

struct FileResult {
    path: String,
    abc: Vec<AbcOffense>,
    used_once: Vec<UsedOnceOffense>,
}

const RUBY_EXTS: [&str; 4] = ["rb", "rake", "ru", "gemspec"];
const RUBY_NAMES: [&str; 6] = [
    "Gemfile",
    "Rakefile",
    "Capfile",
    "Brewfile",
    "Podfile",
    "Fastfile",
];

fn is_ruby_path(p: &std::path::Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) => RUBY_EXTS.contains(&ext),
        None => p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| RUBY_NAMES.contains(&n))
            .unwrap_or(false),
    }
}

fn collect_files(paths: &[String]) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for raw in paths {
        let p = std::path::Path::new(raw);
        if p.is_file() {
            files.push(p.to_path_buf());
        } else {
            let walker = ignore::WalkBuilder::new(p).build();
            for entry in walker.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                    && is_ruby_path(entry.path())
                {
                    files.push(entry.into_path());
                }
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn parse_file(src: &[u8]) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_ruby::LANGUAGE.into()).ok()?;
    parser.parse(src, None)
}

fn dump_tree(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let src = fs::read(path)?;
    let tree = parse_file(&src).ok_or("parse failed")?;
    fn esc(s: &str) -> String {
        s.replace('\n', "\\n")
    }
    fn rec(cursor: &mut tree_sitter::TreeCursor, src: &[u8], depth: usize) {
        loop {
            let n = cursor.node();
            let field = cursor.field_name().unwrap_or("");
            let text = n.utf8_text(src).unwrap_or("");
            let text = if text.len() <= 60 { esc(text) } else { format!("{}…", esc(&text[..60])) };
            let prefix = if field.is_empty() { String::new() } else { format!("@{field}: ") };
            println!(
                "{ind}{prefix}{kind} [{row}:{col}] {text}",
                ind = "  ".repeat(depth),
                kind = n.kind(),
                row = n.start_position().row + 1,
                col = n.start_position().column
            );
            if cursor.goto_first_child() {
                rec(cursor, src, depth + 1);
                cursor.goto_parent();
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    let mut cursor = tree.walk();
    rec(&mut cursor, &src, 0);
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.dump_tree {
        let Some(path) = cli.paths.first() else {
            eprintln!("--dump-tree requires a file path");
            return ExitCode::from(2);
        };
        return match dump_tree(path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        };
    }

    if cli.paths.is_empty() {
        eprintln!("no paths given");
        return ExitCode::from(2);
    }

    // max threshold is threaded through via closure capture of parsed CLI
    let only = cli.only.clone();
    let files = collect_files(&cli.paths);
    let mut results: Vec<FileResult> = files
        .par_iter()
        .map(|p| analyze_one_with_max(p, only.as_deref(), cli.max_abc))
        .collect();
    results.sort_by(|x, y| x.path.cmp(&y.path));

    let total_abc: usize = results.iter().map(|r| r.abc.len()).sum();
    let total_used: usize = results.iter().map(|r| r.used_once.len()).sum();

    match cli.format.as_str() {
        "json" => print_json(&files.len(), &results),
        _ => print_text(&results, cli.max_abc),
    }

    if total_abc + total_used > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Parsed `rubocop:disable` / `rubocop:enable` directives of one file.
struct Directives {
    /// lines carrying a trailing disable naming Metrics/AbcSize (or all cops)
    abc_lines: std::collections::HashSet<usize>,
    /// line ranges (inclusive) where Metrics/AbcSize is disabled
    abc_ranges: Vec<(usize, usize)>,
    /// same two, for disables with no cop list (suppress everything)
    all_lines: std::collections::HashSet<usize>,
    all_ranges: Vec<(usize, usize)>,
}

fn cop_names(after: &str) -> Vec<String> {
    after
        .split(',')
        .map(|s| s.trim().trim_start_matches(':').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_directives(src: &str) -> Directives {
    let mut d = Directives {
        abc_lines: std::collections::HashSet::new(),
        abc_ranges: Vec::new(),
        all_lines: std::collections::HashSet::new(),
        all_ranges: Vec::new(),
    };
    let mut pending: Vec<(usize, bool)> = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line_no = i + 1;
        let Some(hash) = raw.find('#') else {
            continue;
        };
        let mut comment = raw[hash..].trim_start_matches('#').trim().to_string();
        // `rubocop:todo` is honored exactly like `rubocop:disable`
        if let Some(rest) = comment.strip_prefix("rubocop:todo") {
            comment = format!("rubocop:disable{}", rest);
        }
        let comment = comment.as_str();
        let enable = comment.strip_prefix("rubocop:enable");
        let disable = comment.strip_prefix("rubocop:disable");
        if enable.is_some() {
            for (_, targets_abc) in pending.drain(..) {
                let ranges = if targets_abc {
                    &mut d.abc_ranges
                } else {
                    &mut d.all_ranges
                };
                for r in ranges.iter_mut() {
                    if r.1 == usize::MAX {
                        r.1 = line_no.saturating_sub(1);
                    }
                }
            }
            continue;
        }
        let Some(after) = disable else {
            continue;
        };
        // `rubocop:disable-next` is not a RuboCop directive — ignore it
        if after.starts_with('-') {
            continue;
        }
        let names = cop_names(after.trim());
        // Empty cop list = all cops; a bare `Metrics` namespace disable
        // also covers AbcSize.
        let mentions_abc =
            names.iter().any(|n| n == "Metrics/AbcSize" || n == "Metrics");
        let relevant = names.is_empty() || mentions_abc;
        let trailing = !raw[..hash].trim().is_empty();
        if trailing {
            if names.is_empty() {
                d.all_lines.insert(line_no);
            } else if mentions_abc {
                d.abc_lines.insert(line_no);
            }
        } else if names.is_empty() {
            pending.push((line_no, false));
            d.all_ranges.push((line_no + 1, usize::MAX));
        } else if relevant {
            pending.push((line_no, true));
            d.abc_ranges.push((line_no + 1, usize::MAX));
        }
    }
    d
}
fn analyze_one_with_max(path: &std::path::PathBuf, only: Option<&str>, max: f64) -> FileResult {
    let empty = FileResult {
        path: path.display().to_string(),
        abc: Vec::new(),
        used_once: Vec::new(),
    };
    let Ok(src_bytes) = fs::read(path) else {
        return empty;
    };
    let Some(tree) = parse_file(&src_bytes) else {
        return empty;
    };
    let fm = build(src_bytes, tree);
    let do_abc = only.is_none_or(|o| o == "abc");
    let do_used = only.is_none_or(|o| o == "used-once");
    let text = String::from_utf8_lossy(fm.src.as_slice());
    let dirs = parse_directives(&text);
    let abc_suppressed = |line: usize| {
        dirs.abc_lines.contains(&line)
            || dirs.all_lines.contains(&line)
            || dirs
                .abc_ranges
                .iter()
                .chain(dirs.all_ranges.iter())
                .any(|r| r.0 <= line && line <= r.1)
    };
    let used_suppressed = |line: usize| dirs.all_lines.contains(&line)
        || dirs.all_ranges.iter().any(|r| r.0 <= line && line <= r.1);
    FileResult {
        path: empty.path,
        abc: if do_abc {
            abc::analyze(&fm, max)
                .into_iter()
                .filter(|o| !abc_suppressed(o.line))
                .collect()
        } else {
            Vec::new()
        },
        used_once: if do_used {
            used_once::analyze(&fm)
                .into_iter()
                .filter(|o| !used_suppressed(o.line))
                .collect()
        } else {
            Vec::new()
        },
    }
}

fn print_text(results: &[FileResult], max: f64) {
    for r in results {
        for o in &r.abc {
            println!(
                "{}:{}:{}: C: Metrics/AbcSize: Assignment Branch Condition size for `{}` is too high. [{} {}/{}]",
                r.path,
                o.line,
                o.column,
                o.name,
                o.vector,
                abc::g4(o.score),
                abc::g4(max)
            );
        }
        for o in &r.used_once {
            println!(
                "{}:{}:{}: W: UsedOnce: variable `{}` is assigned once and read once — consider inlining",
                r.path, o.line, o.column, o.name
            );
        }
    }
    println!(
        "{} files, {} abc offenses, {} used-once offenses",
        results.len(),
        results.iter().map(|r| r.abc.len()).sum::<usize>(),
        results.iter().map(|r| r.used_once.len()).sum::<usize>()
    );
}

fn print_json(file_count: &usize, results: &[FileResult]) {
    #[derive(Serialize)]
    struct Out<'a> {
        files: usize,
        diagnostics: Vec<Diagnostic>,
        phantom: std::marker::PhantomData<&'a ()>,
    }
    let mut diags = Vec::new();
    for r in results {
        for o in &r.abc {
            diags.push(Diagnostic {
                file: r.path.clone(),
                line: o.line,
                column: o.column,
                severity: "C",
                rule: "Metrics/AbcSize",
                message: format!(
                    "Assignment Branch Condition size for `{}` is too high. [{} {}]",
                    o.name,
                    o.vector,
                    abc::g4(o.score)
                ),
                score: Some(o.score),
                vector: Some(o.vector.clone()),
            });
        }
        for o in &r.used_once {
            diags.push(Diagnostic {
                file: r.path.clone(),
                line: o.line,
                column: o.column,
                severity: "W",
                rule: "UsedOnce",
                message: format!(
                    "variable `{}` is assigned once and read once — consider inlining",
                    o.name
                ),
                score: None,
                vector: None,
            });
        }
    }
    let out = Out {
        files: *file_count,
        diagnostics: diags,
        phantom: std::marker::PhantomData,
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}

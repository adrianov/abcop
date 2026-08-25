//! Human and JSON rendering of scan results.

use crate::abc;
use crate::modulesize;
use crate::abc::AbcOffense;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

#[derive(serde::Serialize)]
pub(crate) struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub severity: &'static str,
    pub rule: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<String>,
}

pub(crate) struct FileResult {
    pub path: String,
    pub abc: Vec<AbcOffense>,
    pub used_once: Vec<UsedOnceOffense>,
    pub never_used: Vec<NeverUsedOffense>,
    pub oversize: Option<usize>,
}

fn print_module_size(r: &FileResult) {
    if let Some(lines) = r.oversize {
        println!(
            "{}: W: ModuleSize: {} lines (>= {}) -- extract a coherent subunit",
            r.path,
            lines,
            modulesize::MAX_LINES
        );
    }
}

pub fn print_text(
    results: &[FileResult],
    max: f64,
    elapsed: std::time::Duration,
) {
    for r in results {
        for o in &r.abc {
            println!(
                "{}:{}:{}: C: Metrics/AbcSize: Assignment Branch Condition size for `{}` is too high. [{} {}/{}]",
                r.path, o.line, o.column, o.name, o.vector, abc::g4(o.score), abc::g4(max)
            );
        }
        for o in &r.used_once {
            println!(
                "{}:{}:{}: W: UsedOnce: variable `{}` is assigned once and read once -- consider inlining",
                r.path, o.line, o.column, o.name
            );
        }
        for o in &r.never_used {
            println!(
                "{}:{}:{}: W: NeverUsed: variable `{}` is assigned but never used",
                r.path, o.line, o.column, o.name
            );
        }
        print_module_size(r);
    }
    println!(
        "{} files analysed in {:.2}s, {} abc offenses, {} used-once offenses, {} never-used warnings, {} module-size warnings",
        results.len(),
        elapsed.as_secs_f64(),
        results.iter().map(|r| r.abc.len()).sum::<usize>(),
        results.iter().map(|r| r.used_once.len()).sum::<usize>(),
        results.iter().map(|r| r.never_used.len()).sum::<usize>(),
        results.iter().filter_map(|r| r.oversize).count()
    );
}

pub fn print_json(
    file_count: &usize,
    results: &[FileResult],
    elapsed: std::time::Duration,
) {
    let mut diags = Vec::new();
    for r in results {
        if let Some(lines) = r.oversize {
            diags.push(Diagnostic {
                file: r.path.clone(),
                line: lines,
                column: 0,
                severity: "W",
                rule: "ModuleSize",
                message: format!(
                    "module has {} lines (>= {}) -- extract a coherent subunit",
                    lines,
                    modulesize::MAX_LINES
                ),
                score: None,
                vector: None,
            });
        }
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
        for o in &r.never_used {
            diags.push(Diagnostic {
                file: r.path.clone(),
                line: o.line,
                column: o.column,
                severity: "W",
                rule: "NeverUsed",
                message: format!(
                    "variable `{}` is assigned but never used",
                    o.name
                ),
                score: None,
                vector: None,
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
                    "variable `{}` is assigned once and read once -- consider inlining",
                    o.name
                ),
                score: None,
                vector: None,
            });
        }
    }
    let out = JsonOut {
        files: *file_count,
        elapsed_ms: elapsed.as_millis(),
        diagnostics: &diags,
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}

#[derive(serde::Serialize)]
struct JsonOut<'a> {
    files: usize,
    elapsed_ms: u128,
    diagnostics: &'a Vec<Diagnostic>,
}

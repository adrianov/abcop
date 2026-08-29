//! Human and JSON rendering of scan results.

use crate::abc;
use crate::abc::AbcOffense;
use crate::modulesize::{self, ModuleAbc};
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
    pub module_abc: Option<ModuleAbc>,
}

fn print_module_abc(r: &FileResult) {
    if let Some(m) = &r.module_abc {
        println!(
            "{}: W: Metrics/ModuleAbcSize: Assignment Branch Condition size for module is too high. [{} {}/{}] -- extract a coherent subunit",
            r.path,
            m.vector,
            abc::g4(m.score),
            abc::g4(modulesize::MAX_ABC)
        );
    }
}

pub fn print_text(results: &[FileResult], max: f64, elapsed: std::time::Duration) {
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
        print_module_abc(r);
    }
    println!(
        "{} files analysed in {}, {} abc offenses, {} used-once offenses, {} never-used warnings, {} module-abc warnings",
        results.len(),
        summary_secs(elapsed),
        results.iter().map(|r| r.abc.len()).sum::<usize>(),
        results.iter().map(|r| r.used_once.len()).sum::<usize>(),
        results.iter().map(|r| r.never_used.len()).sum::<usize>(),
        results.iter().filter_map(|r| r.module_abc.as_ref()).count()
    );
}

/// Seconds with enough decimals that the fraction always carries a nonzero
/// digit (`0.003s` instead of `0.00s`); two decimals once measurable there.
fn summary_secs(elapsed: std::time::Duration) -> String {
    let s = elapsed.as_secs_f64();
    let d = if s > 0.0 {
        ((-s.log10()).ceil() as usize).max(2)
    } else {
        2
    };
    format!("{:.*}s", d, s)
}

pub fn print_json(file_count: &usize, results: &[FileResult], elapsed: std::time::Duration) {
    let diags: Vec<Diagnostic> = results.iter().flat_map(result_diagnostics).collect();
    let out = JsonOut {
        files: *file_count,
        elapsed_ms: elapsed.as_millis(),
        diagnostics: &diags,
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}

/// Every diagnostic one file contributes, in rule order.
fn result_diagnostics(r: &FileResult) -> Vec<Diagnostic> {
    let mut diags: Vec<_> = r.module_abc.iter().map(|m| module_abc_diag(r, m)).collect();
    diags.extend(r.abc.iter().map(|o| abc_diag(r, o)));
    diags.extend(r.never_used.iter().map(|o| never_used_diag(r, o)));
    diags.extend(r.used_once.iter().map(|o| used_once_diag(r, o)));
    diags
}

fn module_abc_diag(r: &FileResult, m: &ModuleAbc) -> Diagnostic {
    Diagnostic {
        file: r.path.clone(),
        line: 1,
        column: 0,
        severity: "W",
        rule: "Metrics/ModuleAbcSize",
        message: format!(
            "Assignment Branch Condition size for module is too high. [{} {}] -- extract a coherent subunit",
            m.vector,
            abc::g4(m.score)
        ),
        score: Some(m.score),
        vector: Some(m.vector.clone()),
    }
}

fn abc_diag(r: &FileResult, o: &AbcOffense) -> Diagnostic {
    Diagnostic {
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
    }
}

fn never_used_diag(r: &FileResult, o: &NeverUsedOffense) -> Diagnostic {
    Diagnostic {
        file: r.path.clone(),
        line: o.line,
        column: o.column,
        severity: "W",
        rule: "NeverUsed",
        message: format!("variable `{}` is assigned but never used", o.name),
        score: None,
        vector: None,
    }
}

fn used_once_diag(r: &FileResult, o: &UsedOnceOffense) -> Diagnostic {
    Diagnostic {
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
    }
}

#[derive(serde::Serialize)]
struct JsonOut<'a> {
    files: usize,
    elapsed_ms: u128,
    diagnostics: &'a Vec<Diagnostic>,
}

#[cfg(test)]
mod tests {
    use super::summary_secs;

    #[test]
    fn summary_time_keeps_a_significant_fraction_digit() {
        assert_eq!(
            summary_secs(std::time::Duration::from_secs_f64(41.67)),
            "41.67s"
        );
        assert_eq!(
            summary_secs(std::time::Duration::from_secs_f64(0.09)),
            "0.09s"
        );
        assert_eq!(
            summary_secs(std::time::Duration::from_secs_f64(0.0031)),
            "0.003s"
        );
        assert_eq!(
            summary_secs(std::time::Duration::from_secs_f64(0.000_512)),
            "0.0005s"
        );
    }
}

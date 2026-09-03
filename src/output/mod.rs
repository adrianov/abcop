//! Human, JSON and JSONL rendering of scan results.

mod json;

use std::cmp::Ordering;

use crate::abc;
use crate::abc::AbcOffense;
use crate::modulesize::ModuleAbc;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

pub(crate) use json::JsonStream;

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

impl FileResult {
    pub(crate) fn is_clean(&self) -> bool {
        self.abc.is_empty()
            && self.used_once.is_empty()
            && self.never_used.is_empty()
            && self.module_abc.is_none()
    }
}

/// Aggregate counts for the trailing summary line.
#[derive(Default)]
pub(crate) struct RunStats {
    pub files: usize,
    pub abc: usize,
    pub used_once: usize,
    pub never_used: usize,
    pub module_abc: usize,
    pub dirty: bool,
}

impl RunStats {
    pub(crate) fn add(&mut self, r: &FileResult) {
        self.files += 1;
        self.abc += r.abc.len();
        self.used_once += r.used_once.len();
        self.never_used += r.never_used.len();
        self.module_abc += usize::from(r.module_abc.is_some());
        self.dirty |= !r.is_clean();
    }

    pub(crate) fn from_results(results: &[FileResult]) -> Self {
        let mut s = Self::default();
        for r in results {
            s.add(r);
        }
        s
    }
}

/// One printable text line plus the score used when sorting worst-first.
struct TextLine {
    score: Option<f64>,
    severity: &'static str,
    file: String,
    line: usize,
    column: usize,
    rule: &'static str,
    text: String,
}

fn print_module_abc(r: &FileResult, max_module: f64) {
    if let Some(m) = &r.module_abc {
        println!("{}", module_abc_text(r, m, max_module));
    }
}

fn module_abc_text(r: &FileResult, m: &ModuleAbc, max_module: f64) -> String {
    format!(
        "{}: W: Metrics/ModuleAbcSize: Assignment Branch Condition size for module is too high. [{} {}/{}] -- extract a coherent subunit",
        r.path,
        m.vector,
        abc::g4(m.score),
        abc::g4(max_module)
    )
}

fn abc_text(r: &FileResult, o: &AbcOffense, max: f64) -> String {
    format!(
        "{}:{}:{}: C: Metrics/AbcSize: Assignment Branch Condition size for `{}` is too high. [{} {}/{}]",
        r.path,
        o.line,
        o.column,
        o.name,
        o.vector,
        abc::g4(o.score),
        abc::g4(max)
    )
}

fn used_once_text(r: &FileResult, o: &UsedOnceOffense) -> String {
    format!(
        "{}:{}:{}: W: UsedOnce: variable `{}` is assigned once and read once -- consider inlining",
        r.path, o.line, o.column, o.name
    )
}

fn never_used_text(r: &FileResult, o: &NeverUsedOffense) -> String {
    let hint = if o.keep_init {
        " -- consider dropping the binding and keeping the initializer"
    } else {
        ""
    };
    format!(
        "{}:{}:{}: W: NeverUsed: variable `{}` is assigned but never used{hint}",
        r.path, o.line, o.column, o.name
    )
}

fn text_lines(results: &[FileResult], limits: abc::Limits) -> Vec<TextLine> {
    let mut lines = Vec::new();
    for r in results {
        for o in &r.abc {
            lines.push(TextLine {
                score: Some(o.score),
                severity: "C",
                file: r.path.clone(),
                line: o.line,
                column: o.column,
                rule: "Metrics/AbcSize",
                text: abc_text(r, o, limits.method),
            });
        }
        for o in &r.used_once {
            lines.push(TextLine {
                score: None,
                severity: "W",
                file: r.path.clone(),
                line: o.line,
                column: o.column,
                rule: "UsedOnce",
                text: used_once_text(r, o),
            });
        }
        for o in &r.never_used {
            lines.push(TextLine {
                score: None,
                severity: "W",
                file: r.path.clone(),
                line: o.line,
                column: o.column,
                rule: "NeverUsed",
                text: never_used_text(r, o),
            });
        }
        if let Some(m) = &r.module_abc {
            lines.push(TextLine {
                score: Some(m.score),
                severity: "W",
                file: r.path.clone(),
                line: 1,
                column: 0,
                rule: "Metrics/ModuleAbcSize",
                text: module_abc_text(r, m, limits.module),
            });
        }
    }
    lines
}

/// Bigger scored findings first; unscored last. Ties break on severity
/// (C before W), then file / line / column / rule for determinism.
fn score_order(
    score_a: Option<f64>,
    sev_a: &str,
    file_a: &str,
    line_a: usize,
    col_a: usize,
    rule_a: &str,
    score_b: Option<f64>,
    sev_b: &str,
    file_b: &str,
    line_b: usize,
    col_b: usize,
    rule_b: &str,
) -> Ordering {
    match (score_a, score_b) {
        (Some(a), Some(b)) => b.partial_cmp(&a).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| severity_rank(sev_a).cmp(&severity_rank(sev_b)))
    .then_with(|| file_a.cmp(file_b))
    .then_with(|| line_a.cmp(&line_b))
    .then_with(|| col_a.cmp(&col_b))
    .then_with(|| rule_a.cmp(rule_b))
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "C" => 0,
        "W" => 1,
        _ => 2,
    }
}

fn sort_text_lines(lines: &mut [TextLine]) {
    lines.sort_by(|a, b| {
        score_order(
            a.score, a.severity, &a.file, a.line, a.column, a.rule, b.score, b.severity,
            &b.file, b.line, b.column, b.rule,
        )
    });
}

/// Emit one file's findings immediately (text mode, no global sort).
pub fn print_file_text(r: &FileResult, limits: abc::Limits) {
    use std::io::Write;
    for o in &r.abc {
        println!("{}", abc_text(r, o, limits.method));
    }
    for o in &r.used_once {
        println!("{}", used_once_text(r, o));
    }
    for o in &r.never_used {
        println!("{}", never_used_text(r, o));
    }
    print_module_abc(r, limits.module);
    let _ = std::io::stdout().flush();
}

pub fn print_summary(stats: &RunStats, elapsed: std::time::Duration) {
    println!(
        "{} files analysed in {}, {} abc offenses, {} used-once offenses, {} never-used warnings, {} module-abc warnings",
        stats.files,
        summary_secs(elapsed),
        stats.abc,
        stats.used_once,
        stats.never_used,
        stats.module_abc
    );
}

/// Buffer every finding, sort highest score first, then emit.
pub fn print_text_sorted(
    results: &[FileResult],
    limits: abc::Limits,
    elapsed: std::time::Duration,
) {
    let mut lines = text_lines(results, limits);
    sort_text_lines(&mut lines);
    for line in &lines {
        println!("{}", line.text);
    }
    print_summary(&RunStats::from_results(results), elapsed);
}

pub use json::{print_file_jsonl, print_json, print_jsonl};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulesize::ModuleAbc;

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

    fn sample_results() -> Vec<FileResult> {
        vec![
            FileResult {
                path: "a.rb".into(),
                abc: vec![AbcOffense {
                    line: 10,
                    end_line: 20,
                    column: 0,
                    name: "small".into(),
                    score: 18.0,
                    vector: "<10, 10, 10>".into(),
                }],
                used_once: vec![UsedOnceOffense {
                    line: 3,
                    column: 2,
                    name: "tmp".into(),
                }],
                never_used: vec![],
                module_abc: None,
            },
            FileResult {
                path: "b.rb".into(),
                abc: vec![AbcOffense {
                    line: 1,
                    end_line: 40,
                    column: 0,
                    name: "big".into(),
                    score: 42.0,
                    vector: "<20, 30, 20>".into(),
                }],
                used_once: vec![],
                never_used: vec![NeverUsedOffense {
                    line: 5,
                    column: 0,
                    name: "dead".into(),
                    keep_init: false,
                }],
                module_abc: Some(ModuleAbc {
                    score: 130.0,
                    vector: "<50, 80, 60>".into(),
                    methods: vec![],
                }),
            },
        ]
    }

    #[test]
    fn sort_by_score_puts_biggest_first() {
        let results = sample_results();
        let mut lines = text_lines(
            &results,
            abc::Limits {
                method: 17.0,
                module: crate::modulesize::MAX_ABC,
            },
        );
        sort_text_lines(&mut lines);
        let rules: Vec<_> = lines.iter().map(|l| l.rule).collect();
        assert_eq!(
            rules,
            [
                "Metrics/ModuleAbcSize",
                "Metrics/AbcSize",
                "Metrics/AbcSize",
                "UsedOnce",
                "NeverUsed",
            ]
        );
        assert_eq!(lines[1].file, "b.rb");
        assert_eq!(lines[2].file, "a.rb");
    }
}

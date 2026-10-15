//! JSON and JSONL rendering: buffered emit and live streaming.

use super::{Diagnostic, FileResult, RunStats, score_order};
use crate::abc;
use crate::abc::AbcOffense;
use crate::modulesize::ModuleAbc;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

/// Buffered JSON emit with findings already ordered highest-score first.
pub fn print_json(
    file_count: &usize,
    results: &[FileResult],
    limits: abc::Limits,
    elapsed: std::time::Duration,
) {
    let mut diags: Vec<Diagnostic> = results
        .iter()
        .flat_map(|r| result_diagnostics(r, limits))
        .collect();
    sort_diagnostics(&mut diags);
    let out = JsonOut {
        diagnostics: &diags,
        files: *file_count,
        elapsed_ms: elapsed.as_millis(),
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}

/// Buffered JSONL emit: one diagnostic object per line, highest-score first.
pub fn print_jsonl(results: &[FileResult], limits: abc::Limits) {
    let mut diags: Vec<Diagnostic> = results
        .iter()
        .flat_map(|r| result_diagnostics(r, limits))
        .collect();
    sort_diagnostics(&mut diags);
    for d in &diags {
        println!("{}", serde_json::to_string(d).unwrap());
    }
}

/// Emit one file's findings as JSON Lines (no global sort).
pub fn print_file_jsonl(r: &FileResult, limits: abc::Limits) {
    write_jsonl(&mut std::io::stdout(), r, limits);
}

/// Streams one JSON document: opens `diagnostics` immediately, appends
/// each file's findings as they finish, then closes with `files` /
/// `elapsed_ms`. Still one object -- key order puts the array first so
/// the document can grow without buffering.
pub(crate) struct JsonStream<W: std::io::Write> {
    out: W,
    first: bool,
}

impl JsonStream<std::io::Stdout> {
    pub(crate) fn begin() -> Self {
        Self::begin_to(std::io::stdout())
    }
}

impl<W: std::io::Write> JsonStream<W> {
    pub(crate) fn begin_to(mut out: W) -> Self {
        let _ = write!(out, "{{\"diagnostics\":[");
        let _ = out.flush();
        Self { out, first: true }
    }

    pub(crate) fn write_file(&mut self, r: &FileResult, limits: abc::Limits) {
        for d in result_diagnostics(r, limits) {
            if !self.first {
                let _ = write!(self.out, ",");
            }
            self.first = false;
            let _ = write!(self.out, "{}", serde_json::to_string(&d).unwrap());
        }
        let _ = self.out.flush();
    }

    pub(crate) fn finish(mut self, stats: &RunStats, elapsed: std::time::Duration) {
        let _ = writeln!(
            self.out,
            "],\"files\":{},\"elapsed_ms\":{}}}",
            stats.files,
            elapsed.as_millis()
        );
    }
}

fn write_jsonl(out: &mut impl std::io::Write, r: &FileResult, limits: abc::Limits) {
    for d in result_diagnostics(r, limits) {
        let _ = writeln!(out, "{}", serde_json::to_string(&d).unwrap());
    }
    let _ = out.flush();
}

fn sort_diagnostics(diags: &mut [Diagnostic]) {
    diags.sort_by(|a, b| {
        score_order(
            a.score, a.severity, &a.file, a.line, a.column, a.rule, b.score, b.severity, &b.file,
            b.line, b.column, b.rule,
        )
    });
}

/// Every diagnostic one file contributes, in rule order.
fn result_diagnostics(r: &FileResult, limits: abc::Limits) -> Vec<Diagnostic> {
    let mut diags: Vec<_> = r
        .module_abc
        .iter()
        .map(|m| module_abc_diag(r, m, limits.module))
        .collect();
    diags.extend(r.abc.iter().map(|o| abc_diag(r, o, limits.method)));
    diags.extend(r.never_used.iter().map(|o| never_used_diag(r, o)));
    diags.extend(r.used_once.iter().map(|o| used_once_diag(r, o)));
    diags
}

fn module_abc_diag(r: &FileResult, m: &ModuleAbc, ceiling: f64) -> Diagnostic {
    Diagnostic {
        file: r.path.clone(),
        line: 1,
        column: 0,
        severity: "W",
        rule: "Metrics/ModuleAbcSize",
        message: crate::modulesize::finding_message(m, ceiling, false),
        score: Some(m.score),
        vector: Some(m.vector.clone()),
    }
}

fn abc_diag(r: &FileResult, o: &AbcOffense, ceiling: f64) -> Diagnostic {
    Diagnostic {
        file: r.path.clone(),
        line: o.line,
        column: o.column,
        severity: "C",
        rule: "Metrics/AbcSize",
        message: format!(
            "Assignment Branch Condition size for `{}` {}. [{} {}]",
            o.name,
            abc::finding_clause(o, ceiling),
            o.vector,
            abc::g4(o.score)
        ),
        score: Some(o.score),
        vector: Some(o.vector.clone()),
    }
}

fn never_used_diag(r: &FileResult, o: &NeverUsedOffense) -> Diagnostic {
    let hint = if o.keep_init {
        " -- consider dropping the binding and keeping the initializer"
    } else {
        ""
    };
    Diagnostic {
        file: r.path.clone(),
        line: o.line,
        column: o.column,
        severity: "W",
        rule: "NeverUsed",
        message: format!("variable `{}` is assigned but never used{hint}", o.name),
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
    diagnostics: &'a Vec<Diagnostic>,
    files: usize,
    elapsed_ms: u128,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abc::AbcOffense;
    use crate::modulesize::ModuleAbc;
    use crate::never_used::NeverUsedOffense;
    use crate::used_once::UsedOnceOffense;

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
                    start_byte: 0,
                    end_byte: 0,
                    prior: None,
                }],
                used_once: vec![UsedOnceOffense {
                    line: 3,
                    column: 2,
                    name: "tmp".into(),
                }],
                never_used: vec![],
                module_abc: None,
                module_score: 0.0,
                units: Vec::new(),
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
                    start_byte: 0,
                    end_byte: 0,
                    prior: None,
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
                    prior: None,
                }),
                module_score: 130.0,
                units: Vec::new(),
            },
        ]
    }

    fn test_limits() -> abc::Limits {
        abc::Limits {
            method: 17.0,
            module: crate::modulesize::MAX_ABC,
            increased: true,
        }
    }

    #[test]
    fn sort_diagnostics_matches_text_order() {
        let limits = test_limits();
        let mut diags: Vec<_> = sample_results()
            .iter()
            .flat_map(|r| result_diagnostics(r, limits))
            .collect();
        sort_diagnostics(&mut diags);
        assert_eq!(diags[0].rule, "Metrics/ModuleAbcSize");
        assert_eq!(diags[0].score, Some(130.0));
        assert_eq!(diags[1].score, Some(42.0));
        assert_eq!(diags[2].score, Some(18.0));
        assert!(diags[3].score.is_none());
        assert!(diags[4].score.is_none());
    }

    #[test]
    fn json_stream_builds_one_parseable_object() {
        let results = sample_results();
        let mut buf = Vec::new();
        let mut stream = JsonStream::begin_to(&mut buf);
        let mut stats = RunStats::default();
        for r in &results {
            stream.write_file(r, test_limits());
            stats.add(r);
        }
        stream.finish(&stats, std::time::Duration::from_millis(12));
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["files"], 2);
        assert_eq!(v["elapsed_ms"], 12);
        assert_eq!(v["diagnostics"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn jsonl_writes_one_object_per_line() {
        let results = sample_results();
        let mut buf = Vec::new();
        for r in &results {
            write_jsonl(&mut buf, r, test_limits());
        }
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(v.get("rule").is_some());
            assert!(v.get("file").is_some());
        }
        assert!(!text.contains("elapsed_ms"));
    }

    #[test]
    fn json_rise_under_the_method_ceiling_names_that_ceiling() {
        let mut results = sample_results();
        results[0].abc[0].prior = Some(16.28);
        results[0].abc[0].score = 21.28;
        results[0].abc[0].vector = "<2, 20, 7>".into();
        results[0].abc[0].name = "rubyLead".into();
        let msg = &result_diagnostics(&results[0], test_limits())[0].message;
        assert_eq!(
            msg,
            "Assignment Branch Condition size for `rubyLead` rose from 16.28 to 21.28 -- back to 17 or below. [<2, 20, 7> 21.28]"
        );
        let tighter = abc::Limits {
            method: 15.0,
            ..test_limits()
        };
        let msg = &result_diagnostics(&results[0], tighter)[0].message;
        assert!(msg.contains("back to 16.28 or below"), "{msg}");
    }
}

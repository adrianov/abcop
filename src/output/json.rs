//! JSON and JSONL rendering: buffered emit and live streaming.

use super::{Diagnostic, FileResult, RunStats, score_order};
use crate::abc;
use crate::abc::AbcOffense;
use crate::modulesize::ModuleAbc;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

/// Buffered JSON emit with findings already ordered highest-score first.
pub fn print_json(file_count: &usize, results: &[FileResult], elapsed: std::time::Duration) {
    let mut diags: Vec<Diagnostic> = results.iter().flat_map(result_diagnostics).collect();
    sort_diagnostics(&mut diags);
    let out = JsonOut {
        diagnostics: &diags,
        files: *file_count,
        elapsed_ms: elapsed.as_millis(),
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}

/// Buffered JSONL emit: one diagnostic object per line, highest-score first.
pub fn print_jsonl(results: &[FileResult]) {
    let mut diags: Vec<Diagnostic> = results.iter().flat_map(result_diagnostics).collect();
    sort_diagnostics(&mut diags);
    for d in &diags {
        println!("{}", serde_json::to_string(d).unwrap());
    }
}

/// Emit one file's findings as JSON Lines (no global sort).
pub fn print_file_jsonl(r: &FileResult) {
    write_jsonl(&mut std::io::stdout(), r);
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

    pub(crate) fn write_file(&mut self, r: &FileResult) {
        for d in result_diagnostics(r) {
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

fn write_jsonl(out: &mut impl std::io::Write, r: &FileResult) {
    for d in result_diagnostics(r) {
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
    fn sort_diagnostics_matches_text_order() {
        let mut diags: Vec<_> = sample_results()
            .iter()
            .flat_map(result_diagnostics)
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
            stream.write_file(r);
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
            write_jsonl(&mut buf, r);
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
}

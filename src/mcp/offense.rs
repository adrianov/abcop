//! LSP-shaped offense JSON (matches RuboCop / rrubocop MCP diagnostics).

use serde_json::{json, Value};

use crate::abc::AbcOffense;
use crate::modulesize::ModuleAbc;
use crate::never_used::NeverUsedOffense;
use crate::output::FileResult;
use crate::used_once::UsedOnceOffense;

/// Map abcop text severity letter to LSP DiagnosticSeverity.
fn lsp_severity(sev: &str) -> u8 {
    match sev {
        "C" => 3, // Convention (AbcSize)
        _ => 2,   // Warning
    }
}

fn range(line: usize, column: usize, len: usize) -> Value {
    let line = line.saturating_sub(1);
    let end = column + len.max(1);
    json!({
        "start": { "line": line, "character": column },
        "end": { "line": line, "character": end }
    })
}

fn offense(
    range: Value,
    severity: &str,
    code: &str,
    message: String,
    data: Value,
) -> Value {
    json!({
        "range": range,
        "severity": lsp_severity(severity),
        "source": "abcop",
        "code": code,
        "message": message,
        "data": data
    })
}

fn abc_offense(o: &AbcOffense) -> Value {
    let len = o.name.chars().count().max(1);
    offense(
        range(o.line, o.column, len),
        "C",
        "Metrics/AbcSize",
        format!(
            "Assignment Branch Condition size for `{}` is too high. [{} {}]",
            o.name,
            o.vector,
            crate::abc::g4(o.score)
        ),
        json!({ "correctable": false, "score": o.score, "vector": o.vector }),
    )
}

fn module_offense(m: &ModuleAbc) -> Value {
    offense(
        range(1, 0, 1),
        "W",
        "Metrics/ModuleAbcSize",
        format!(
            "Assignment Branch Condition size for module is too high. [{} {}] -- extract a coherent subunit",
            m.vector,
            crate::abc::g4(m.score)
        ),
        json!({ "correctable": false, "score": m.score, "vector": m.vector }),
    )
}

fn used_once_offense(o: &UsedOnceOffense) -> Value {
    let len = o.name.chars().count().max(1);
    offense(
        range(o.line, o.column, len),
        "W",
        "UsedOnce",
        format!(
            "variable `{}` is assigned once and read once -- consider inlining",
            o.name
        ),
        json!({ "correctable": false }),
    )
}

fn never_used_offense(o: &NeverUsedOffense) -> Value {
    let hint = if o.keep_init {
        " -- consider dropping the binding and keeping the initializer"
    } else {
        ""
    };
    let len = o.name.chars().count().max(1);
    offense(
        range(o.line, o.column, len),
        "W",
        "NeverUsed",
        format!("variable `{}` is assigned but never used{hint}", o.name),
        json!({ "correctable": false }),
    )
}

/// Every finding for one file as LSP-shaped offenses.
pub(crate) fn to_lsp_offenses(r: &FileResult) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(m) = &r.module_abc {
        out.push(module_offense(m));
    }
    out.extend(r.abc.iter().map(abc_offense));
    out.extend(r.never_used.iter().map(never_used_offense));
    out.extend(r.used_once.iter().map(used_once_offense));
    out
}

pub(crate) fn offenses_json(r: &FileResult) -> String {
    serde_json::to_string(&to_lsp_offenses(r)).unwrap_or_else(|_| "[]".into())
}

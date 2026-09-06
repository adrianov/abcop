//! Compact offense JSON for MCP (agent-oriented, not full LSP diagnostics).
//!
//! No `source` / `severity` / nested `range` — those repeat on every row and
//! waste tokens. Line/column are 1-based like CLI text and `--format json`.

use serde_json::{json, Value};

use crate::abc::AbcOffense;
use crate::modulesize::ModuleAbc;
use crate::never_used::NeverUsedOffense;
use crate::output::FileResult;
use crate::used_once::UsedOnceOffense;

fn offense(line: usize, column: usize, code: &str, message: String) -> Value {
    json!({
        "code": code,
        "line": line,
        "column": column,
        "message": message,
    })
}

fn abc_offense(o: &AbcOffense) -> Value {
    let mut v = offense(
        o.line,
        o.column,
        "Metrics/AbcSize",
        format!(
            "Assignment Branch Condition size for `{}` is too high. [{} {}]",
            o.name,
            o.vector,
            crate::abc::g4(o.score)
        ),
    );
    v["score"] = json!(o.score);
    v["vector"] = json!(o.vector);
    v
}

fn module_offense(m: &ModuleAbc) -> Value {
    let mut v = offense(
        1,
        0,
        "Metrics/ModuleAbcSize",
        format!(
            "Assignment Branch Condition size for module is too high. [{} {}] -- extract a coherent subunit",
            m.vector,
            crate::abc::g4(m.score)
        ),
    );
    v["score"] = json!(m.score);
    v["vector"] = json!(m.vector);
    v
}

fn used_once_offense(o: &UsedOnceOffense) -> Value {
    offense(
        o.line,
        o.column,
        "UsedOnce",
        format!(
            "variable `{}` is assigned once and read once -- consider inlining",
            o.name
        ),
    )
}

fn never_used_offense(o: &NeverUsedOffense) -> Value {
    let hint = if o.keep_init {
        " -- consider dropping the binding and keeping the initializer"
    } else {
        ""
    };
    offense(
        o.line,
        o.column,
        "NeverUsed",
        format!("variable `{}` is assigned but never used{hint}", o.name),
    )
}

/// Every finding for one file as compact offenses.
pub(crate) fn to_offenses(r: &FileResult) -> Vec<Value> {
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
    serde_json::to_string(&to_offenses(r)).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::never_used::NeverUsedOffense;

    #[test]
    fn never_used_offense_is_flat() {
        let r = FileResult {
            path: "t.rb".into(),
            abc: Vec::new(),
            used_once: Vec::new(),
            never_used: vec![NeverUsedOffense {
                line: 2,
                column: 2,
                name: "x".into(),
                keep_init: false,
            }],
            module_abc: None,
        };
        let text = offenses_json(&r);
        assert!(text.contains("\"code\":\"NeverUsed\""));
        assert!(
            text.find("\"line\"").unwrap() < text.find("\"column\"").unwrap(),
            "line before column for LLMs, got {text}"
        );
        assert!(!text.contains("\"source\"") && !text.contains("\"range\"") && !text.contains("\"data\""));
    }
}

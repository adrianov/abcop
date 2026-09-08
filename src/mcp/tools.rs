//! Inspection logic shared by MCP tools.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::abc::Limits;
use crate::modulesize;
use crate::output::FileResult;
use crate::pipeline;
use crate::walker;

use super::offense;
use super::targets;

pub(crate) struct State {
    pub(crate) limits: Limits,
}

impl Default for State {
    fn default() -> Self {
        Self {
            limits: Limits {
                method: 17.0,
                module: modulesize::MAX_ABC,
            },
        }
    }
}

pub(crate) fn inspect(
    state: &State,
    targets: Vec<String>,
    source: Option<String>,
) -> Result<String, String> {
    match source {
        Some(code) => Ok(inspect_inline(state, targets.first().map(String::as_str), &code)),
        None => inspect_paths(state, targets),
    }
}

fn inspect_inline(state: &State, path: Option<&str>, code: &str) -> String {
    offense::offenses_json(&pipeline::analyze_src(
        Path::new(path.unwrap_or("example.rb")),
        code.as_bytes(),
        None,
        state.limits,
    ))
}

fn inspect_paths(state: &State, targets: Vec<String>) -> Result<String, String> {
    let files = target_files(&targets)?;
    Ok(pack_offenses(
        &files,
        &files
            .iter()
            .map(|file| {
                (
                    file.display().to_string(),
                    pipeline::analyze_one(file, None, state.limits, None, None),
                )
            })
            .collect::<Vec<_>>(),
    ))
}

fn pack_offenses(targets: &[PathBuf], all: &[(String, FileResult)]) -> String {
    let offense_count: usize = all.iter().map(|(_, r)| finding_count(r)).sum();
    let files: Vec<Value> = all
        .iter()
        .filter(|(_, r)| !r.is_clean())
        .map(|(path, r)| {
            json!({
                "path": path,
                "offenses": offense::to_offenses(r)
            })
        })
        .collect();
    json!({
        "files": files,
        "summary": {
            "target_file_count": targets.len(),
            "offense_count": offense_count
        }
    })
    .to_string()
}

fn finding_count(r: &FileResult) -> usize {
    r.abc.len()
        + r.used_once.len()
        + r.never_used.len()
        + usize::from(r.module_abc.is_some())
}

fn target_files(targets: &[String]) -> Result<Vec<PathBuf>, String> {
    targets::validate_roots(targets)?;
    Ok(walker::collect_files(targets, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_dead_rb() -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("abcop_mcp_paths_unit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.rb");
        let b = dir.join("b.rb");
        std::fs::write(&a, "def a\n  x = 1\nend\n").unwrap();
        std::fs::write(&b, "def b\n  y = 1\nend\n").unwrap();
        (dir, a, b)
    }

    #[test]
    fn paths_array_inspects_files() {
        let (dir, a, b) = two_dead_rb();
        let json = inspect(
            &State::default(),
            vec![a.display().to_string(), b.display().to_string()],
            None,
        )
        .unwrap();
        assert!(json.contains("\"target_file_count\":2"), "{json}");
        assert!(json.contains("NeverUsed"), "{json}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! Inspection logic shared by MCP tools.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::abc::Limits;
use crate::modulesize;
use crate::output::FileResult;
use crate::pipeline;
use crate::walker;

use super::offense;

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
    path: Option<String>,
    source: Option<String>,
) -> Result<String, String> {
    match source {
        Some(code) => Ok(inspect_inline(state, path, &code)),
        None => inspect_paths(state, path),
    }
}

fn inspect_inline(state: &State, path: Option<String>, code: &str) -> String {
    offense::offenses_json(&pipeline::analyze_src(
        Path::new(path.as_deref().unwrap_or("example.rb")),
        code.as_bytes(),
        None,
        state.limits,
    ))
}

fn inspect_paths(state: &State, path: Option<String>) -> Result<String, String> {
    let files = target_files(path)?;
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
                "offenses": offense::to_lsp_offenses(r)
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

fn target_files(path: Option<String>) -> Result<Vec<PathBuf>, String> {
    let root = path.unwrap_or_else(|| ".".into());
    if !Path::new(&root).exists() {
        return Err(format!("No such file or directory: {root}"));
    }
    Ok(walker::collect_files(&[root], false))
}

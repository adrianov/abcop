//! Shared MCP path-target parsing: `path` / `paths`, home refusal, existence checks.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;

/// One file/dir string or a list (LLMs often put an array in `path`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

/// Merge `path` + `paths`, drop blanks.
pub(crate) fn merge(path: Option<OneOrMany>, paths: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    match path {
        Some(OneOrMany::One(s)) => push_nonempty(&mut out, s),
        Some(OneOrMany::Many(many)) => {
            for s in many {
                push_nonempty(&mut out, s);
            }
        }
        None => {}
    }
    for p in paths {
        push_nonempty(&mut out, p);
    }
    out
}

fn push_nonempty(out: &mut Vec<String>, s: String) {
    if !s.trim().is_empty() {
        out.push(s);
    }
}

pub(crate) fn validate_roots(targets: &[String]) -> Result<(), String> {
    if targets.is_empty() {
        return Err("Provide `path` or `paths` (absolute file or directory).".into());
    }
    for raw in targets {
        refuse_home_walk(Path::new(raw))?;
        if !Path::new(raw).exists() {
            return Err(format!("No such file or directory: {raw}"));
        }
    }
    Ok(())
}

/// Refuse scanning `$HOME` (MCP cwd often lands there when client config is wrong).
fn refuse_home_walk(root: &Path) -> Result<(), String> {
    refuse_home_walk_at(
        root,
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    )
}

fn refuse_home_walk_at(root: &Path, cwd: &Path) -> Result<(), String> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Ok(());
    };
    let resolved = if root == Path::new(".") {
        cwd.to_path_buf()
    } else {
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
    };
    let home = home.canonicalize().unwrap_or(home);
    if resolved == home {
        return Err(
            "Refusing to scan the home directory; pass a project file or directory via `path` / `paths`."
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_path_array_and_paths() {
        let got = merge(
            Some(OneOrMany::Many(vec!["a.swift".into(), "".into()])),
            vec!["b.swift".into()],
        );
        assert_eq!(got, vec!["a.swift", "b.swift"]);
    }

    #[test]
    fn empty_targets_error() {
        let err = validate_roots(&[]).unwrap_err();
        assert!(err.contains("`path` or `paths`"), "{err}");
    }

    #[test]
    fn home_directory_refused() {
        let err = validate_roots(&[std::env::var("HOME").expect("HOME")]).unwrap_err();
        assert!(err.contains("home directory"), "{err}");
    }

    #[test]
    fn dot_refused_when_cwd_is_home() {
        let err = refuse_home_walk_at(
            Path::new("."),
            &PathBuf::from(std::env::var("HOME").expect("HOME")),
        )
        .unwrap_err();
        assert!(err.contains("home directory"), "{err}");
    }
}

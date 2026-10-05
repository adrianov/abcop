//! The single user config file, and which directory a scanned path belongs to.

use std::path::{Path, PathBuf};

/// Directory arguments stay; a file argument anchors at its parent.
pub(crate) fn anchor_paths(paths: &[String]) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|raw| {
            let path = PathBuf::from(raw);
            if path.is_file() {
                path.parent().map(Path::to_path_buf).unwrap_or(path)
            } else {
                path
            }
        })
        .collect()
}

/// `$ABCOP_CONFIG`, else `$XDG_CONFIG_HOME/abcop`, else `~/.config/abcop`.
pub(super) fn config_file() -> Option<PathBuf> {
    config_file_from(
        non_empty_env("ABCOP_CONFIG").as_deref(),
        non_empty_env("XDG_CONFIG_HOME").as_deref(),
        non_empty_env("HOME").as_deref(),
    )
}

pub(super) fn config_file_from(
    over: Option<&str>,
    xdg: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(path) = over {
        return Some(PathBuf::from(path));
    }
    if let Some(dir) = xdg {
        return Some(PathBuf::from(dir).join("abcop"));
    }
    home.map(|dir| PathBuf::from(dir).join(".config").join("abcop"))
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// `ignore` values from the user config. Missing file means no patterns.
pub(super) fn ignore_patterns(file: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    text.lines().filter_map(ignore_value).collect()
}

fn ignore_value(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let rest = line.strip_prefix("ignore")?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t', '=']) {
        return None;
    }
    let value = rest.trim().trim_start_matches('=').trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(super) fn resolve_root(path: &Path, anchors: &[PathBuf]) -> PathBuf {
    let path = canonical(path);
    git_workdir(&path)
        .or_else(|| shortest_anchor(&path, anchors))
        .unwrap_or_else(|| start_dir(&path))
}

fn git_workdir(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    let repo = git2::Repository::discover(start).ok()?;
    repo.workdir().map(canonical)
}

fn shortest_anchor(path: &Path, anchors: &[PathBuf]) -> Option<PathBuf> {
    anchors
        .iter()
        .filter(|anchor| path.starts_with(anchor))
        .min_by_key(|anchor| anchor.components().count())
        .cloned()
}

pub(super) fn start_dir(path: &Path) -> PathBuf {
    let path = canonical(path);
    if path.is_dir() {
        path
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    }
}

pub(super) fn canonical(path: &Path) -> PathBuf {
    if let Ok(found) = std::fs::canonicalize(path) {
        return found;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

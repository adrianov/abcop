//! One user config file. Options live there; `ignore` lists gitignore
//! patterns applied in every project (relative to that project's root).
//!
//! ```text
//! $XDG_CONFIG_HOME/abcop
//! ~/.config/abcop
//! ```
//!
//! ```text
//! ignore = app/services/legacy/
//! ignore = lib/old.rb
//! ```
//!
//! `$ABCOP_CONFIG` replaces the file path. Nothing is read from the
//! repository. Naming a path does not opt it back in, and `--everything`
//! does not lift this list.

mod locate;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub(crate) use locate::anchor_paths;
use locate::{canonical, config_file, ignore_patterns, resolve_root, start_dir};

struct Cache {
    roots: HashMap<PathBuf, PathBuf>,
    matchers: HashMap<PathBuf, Option<Arc<Gitignore>>>,
}

/// Global ignore patterns from the user config, matched per project root.
pub(crate) struct Excludes {
    patterns: Vec<String>,
    anchors: Vec<PathBuf>,
    cache: Mutex<Cache>,
}

impl Excludes {
    /// User config for this process, anchored at the directories being scanned.
    pub(crate) fn load(anchors: &[PathBuf]) -> Self {
        Self::from_file(config_file(), anchors)
    }

    fn from_file(file: Option<PathBuf>, anchors: &[PathBuf]) -> Self {
        Self {
            patterns: file.as_deref().map(ignore_patterns).unwrap_or_default(),
            anchors: anchors.iter().map(|path| canonical(path)).collect(),
            cache: Mutex::new(Cache {
                roots: HashMap::new(),
                matchers: HashMap::new(),
            }),
        }
    }

    /// True when `path` matches a global ignore pattern (a directory
    /// pattern covers files inside it).
    pub(crate) fn skips(&self, path: &Path) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let root = self.cached_root(path);
        let Some(gi) = self.matcher(&root) else {
            return false;
        };
        let path = canonical(path);
        path.starts_with(&root)
            && gi
                .matched_path_or_any_parents(&path, path.is_dir())
                .is_ignore()
    }

    fn cached_root(&self, path: &Path) -> PathBuf {
        let start = start_dir(path);
        if let Some(hit) = self.lock().roots.get(&start).cloned() {
            return hit;
        }
        let root = resolve_root(path, &self.anchors);
        self.lock().roots.insert(start, root.clone());
        root
    }

    fn matcher(&self, root: &Path) -> Option<Arc<Gitignore>> {
        if let Some(hit) = self.lock().matchers.get(root) {
            return hit.clone();
        }
        let built = build_matcher(&self.patterns, root);
        self.lock()
            .matchers
            .insert(root.to_path_buf(), built.clone());
        built
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn build_matcher(patterns: &[String], root: &Path) -> Option<Arc<Gitignore>> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        let _ = builder.add_line(None, pattern);
    }
    let gi = builder.build().ok()?;
    (!gi.is_empty()).then(|| Arc::new(gi))
}

#[cfg(test)]
mod tests;

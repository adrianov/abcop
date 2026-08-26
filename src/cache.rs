//! Content-addressed result cache, modeled after RuboCop's result cache:
//! per-source-file JSON entries keyed by a hash of the file contents plus
//! everything that influences diagnostics (tool version, rule-set revision,
//! threshold, selected checks, and the file path itself).

use std::fs;
use std::path::{Path, PathBuf};

/// Bump whenever counting rules or output shape change so stale entries are
/// never served.
pub const RULES_REV: u32 = 2;
const MAX_ENTRIES: usize = 20_000;

pub struct Cache {
    dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedFile {
    abc: Vec<crate::abc::AbcOffense>,
    used_once: Vec<crate::used_once::UsedOnceOffense>,
    never_used: Vec<crate::never_used::NeverUsedOffense>,
    oversize: Option<usize>,
}

pub type CachedDiags = (
    Vec<crate::abc::AbcOffense>,
    Vec<crate::used_once::UsedOnceOffense>,
    Vec<crate::never_used::NeverUsedOffense>,
    Option<usize>,
);

impl Cache {
    /// Cache directory: `$ABCOP_CACHE_DIR` if set, otherwise the user-wide
    /// XDG cache dir (`$XDG_CACHE_HOME/abcop`, falling back to
    /// `~/.cache/abcop`). Keys hash the full file path, so entries from
    /// different projects never collide.
    pub fn open(disabled: bool) -> Option<Cache> {
        if disabled {
            return None;
        }
        let base = cache_base()?;
        fs::create_dir_all(&base).ok()?;
        Some(Cache { dir: base })
    }
    pub fn file_key(&self, path: &Path, contents: &[u8], only: Option<&str>, max: f64) -> String {
        use sha2::Digest;
        let path_string = path.display().to_string();
        let mut h = sha2::Sha256::new();
        h.update(env!("CARGO_PKG_VERSION").as_bytes());
        h.update(RULES_REV.to_le_bytes());
        h.update(max.to_le_bytes());
        h.update(only.unwrap_or("").as_bytes());
        h.update(&path_string);
        h.update(contents);
        format!("{:x}", h.finalize())
    }

    pub fn get(&self, key: &str) -> Option<CachedDiags> {
        let bytes = fs::read(self.dir.join(format!("{key}.json"))).ok()?;
        let f: CachedFile = serde_json::from_slice(&bytes).ok()?;
        Some((f.abc, f.used_once, f.never_used, f.oversize))
    }

    pub fn store(
        &self,
        key: &str,
        abc: &[crate::abc::AbcOffense],
        used_once: &[crate::used_once::UsedOnceOffense],
        never_used: &[crate::never_used::NeverUsedOffense],
        oversize: Option<usize>,
    ) {
        let payload = serde_json::json!({
            "abc": abc,
            "used_once": used_once,
            "never_used": never_used,
            "oversize": oversize,
        });
        let _ = fs::write(
            self.dir.join(format!("{key}.json")),
            serde_json::to_vec(&payload).unwrap_or_default(),
        );
    }

    /// Keep the newest MAX_ENTRIES entries; drop the rest.
    pub fn prune(&self) {
        let mut by_age = self.entry_ages();
        if by_age.len() <= MAX_ENTRIES {
            return;
        }
        by_age.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
        for (_, path) in by_age.iter().skip(MAX_ENTRIES) {
            let _ = fs::remove_file(path);
        }
    }

    fn entry_ages(&self) -> Vec<(std::time::SystemTime, PathBuf)> {
        match fs::read_dir(&self.dir) {
            Ok(entries) => entries.flatten().filter_map(entry_age).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// `$ABCOP_CACHE_DIR`, else `$XDG_CACHE_HOME/abcop` when set, else
/// `~/.cache/abcop`.
fn cache_base() -> Option<PathBuf> {
    if let Some(dir) = non_empty_env("ABCOP_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = non_empty_env("XDG_CACHE_HOME") {
        return Some(PathBuf::from(dir).join("abcop"));
    }
    home_cache_dir()
}

/// Variable value without surrounding blanks; unset or blank means absent.
fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn home_cache_dir() -> Option<PathBuf> {
    let home = non_empty_env("HOME")?;
    Some(PathBuf::from(home).join(".cache").join("abcop"))
}

/// Modification time and path of one cache entry; unreadable entries drop out.
fn entry_age(e: fs::DirEntry) -> Option<(std::time::SystemTime, PathBuf)> {
    let m = e.metadata().ok()?.modified().ok()?;
    Some((m, e.path()))
}

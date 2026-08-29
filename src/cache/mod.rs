//! Content-addressed result cache backed by a single embedded key-value
//! database ([`redb`]): per-source-file JSON entries keyed by a hash of
//! the file contents plus everything that influences diagnostics (tool
//! version, rule-set revision, threshold, selected checks, and the file
//! path itself).
//!
//! One `cache.redb` file replaces the historical one-JSON-file-per-entry
//! layout. Commits run at `Durability::None`: a lint result cache may
//! lose the tail of a crashed run, never correctness -- every entry is
//! content-keyed and self-validating on parse.
//!
//! Split: this facade owns *where* the database lives and *what
//! identity* a key carries; [`store`] owns the table itself (rows in,
//! rows out, oldest out when the budget overflows).

mod store;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use redb::Database;

pub(crate) use store::CachedDiags;
use store::{EntryStore, RULES_REV};

const DB_FILE: &str = "cache.redb";

pub(crate) struct Cache {
    store: EntryStore,
}

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
        Self::open_at(&base)
    }

    pub(crate) fn open_at(base: &Path) -> Option<Cache> {
        std::fs::create_dir_all(base).ok()?;
        let db = Database::create(base.join(DB_FILE)).ok()?;
        drop_legacy_entries(base);
        Some(Cache {
            store: EntryStore::new(db),
        })
    }

    /// Identity of one file's diagnostics: tool version, rule-set
    /// revision, thresholds and the on-disk path all participate so two
    /// runs never share an entry across incompatible settings.
    pub(crate) fn file_key(
        &self,
        path: &Path,
        contents: &[u8],
        only: Option<&str>,
        limits: crate::abc::Limits,
    ) -> String {
        let path = path.display().to_string();
        let rev = RULES_REV.to_le_bytes();
        let method = limits.method.to_le_bytes();
        let module = limits.module.to_le_bytes();
        hash_parts(&[
            env!("CARGO_PKG_VERSION").as_bytes(),
            &rev,
            &method,
            &module,
            only.unwrap_or("").as_bytes(),
            path.as_bytes(),
            contents,
        ])
    }

    pub(crate) fn get(&self, key: &str) -> Option<CachedDiags> {
        self.store.get(key)
    }

    pub(crate) fn store(
        &self,
        key: &str,
        abc: &[crate::abc::AbcOffense],
        used_once: &[crate::used_once::UsedOnceOffense],
        never_used: &[crate::never_used::NeverUsedOffense],
        module_abc: Option<crate::modulesize::ModuleAbc>,
    ) {
        self.store.store(key, abc, used_once, never_used, module_abc)
    }

    /// Keep the newest MAX_ENTRIES entries; drop the rest.
    pub(crate) fn prune(&self) {
        self.store.prune()
    }

    #[cfg(test)]
    pub(super) fn store_ref(&self) -> &EntryStore {
        &self.store
    }
}

fn hash_parts(parts: &[&[u8]]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for p in parts {
        h.update(p);
    }
    format!("{:x}", h.finalize())
}

/// Remove the pre-redb one-JSON-per-entry files left behind in the cache
/// directory. Best effort: leftovers only waste disk space.
fn drop_legacy_entries(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for e in entries.flatten() {
        if e.path().extension().and_then(|e| e.to_str()) == Some("json") {
            let _ = std::fs::remove_file(e.path());
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

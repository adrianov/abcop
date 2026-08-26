//! Content-addressed result cache backed by a single embedded key-value
//! database ([`redb`]): per-source-file JSON entries keyed by a hash of
//! the file contents plus everything that influences diagnostics (tool
//! version, rule-set revision, threshold, selected checks, and the file
//! path itself).
//!
//! One `cache.redb` file replaces the historical one-JSON-file-per-entry
//! layout. Commits run at [`Durability::None`]: a lint result cache may
//! lose the tail of a crashed run, never correctness -- every entry is
//! content-keyed and self-validating on parse.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use redb::ReadableTableMetadata;
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};

/// Bump whenever counting rules or output shape change so stale entries are
/// never served.
pub const RULES_REV: u32 = 11;
const MAX_ENTRIES: usize = 20_000;
const DB_FILE: &str = "cache.redb";

const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");

pub struct Cache {
    db: Database,
}

#[derive(serde::Deserialize)]
struct CachedFile {
    /// Store time in ms since the epoch; drives recency pruning.
    ts: u64,
    abc: Vec<crate::abc::AbcOffense>,
    used_once: Vec<crate::used_once::UsedOnceOffense>,
    never_used: Vec<crate::never_used::NeverUsedOffense>,
    oversize: Option<usize>,
}

/// Borrowing twin of [`CachedFile`] for serialization straight from the
/// analysis result -- no Clone required on the offense types.
#[derive(serde::Serialize)]
struct CachedFileRef<'a> {
    ts: u64,
    abc: &'a [crate::abc::AbcOffense],
    used_once: &'a [crate::used_once::UsedOnceOffense],
    never_used: &'a [crate::never_used::NeverUsedOffense],
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
        Self::open_at(&base)
    }

    pub(crate) fn open_at(base: &Path) -> Option<Cache> {
        std::fs::create_dir_all(base).ok()?;
        let db = Database::create(base.join(DB_FILE)).ok()?;
        drop_legacy_entries(base);
        Some(Cache { db })
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
        let rtx = self.db.begin_read().ok()?;
        let table = rtx.open_table(ENTRIES).ok()?;
        let value = table.get(key).ok()??;
        let f: CachedFile = serde_json::from_slice(value.value()).ok()?;
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
        let payload = CachedFileRef {
            ts: now_ms(),
            abc,
            used_once,
            never_used,
            oversize,
        };
        let Ok(bytes) = serde_json::to_vec(&payload) else {
            return;
        };
        let Ok(mut tx) = self.db.begin_write() else {
            return;
        };
        if tx.set_durability(Durability::None).is_err() {
            return;
        }
        let Ok(mut table) = tx.open_table(ENTRIES) else {
            return;
        };
        if table.insert(key, bytes.as_slice()).is_err() {
            return;
        }
        drop(table);
        let _ = tx.commit();
    }

    /// Keep the newest MAX_ENTRIES entries; drop the rest.
    pub fn prune(&self) {
        let Some(by_age) = self.entries_by_age() else {
            return;
        };
        if by_age.len() <= MAX_ENTRIES {
            return;
        }
        let mut newest_first = by_age;
        newest_first.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
        let stale: Vec<String> = newest_first
            .iter()
            .skip(MAX_ENTRIES)
            .map(|(_, k)| k.clone())
            .collect();
        self.remove_keys(&stale);
    }

    /// `(timestamp, key)` for every parseable entry, in storage order.
    /// Corrupt rows are skipped so one bad entry cannot break pruning.
    fn entries_by_age(&self) -> Option<Vec<(u64, String)>> {
        let rtx = self.db.begin_read().ok()?;
        let table = rtx.open_table(ENTRIES).ok()?;
        let iter = table.iter().ok()?;
        Some(
            iter.flatten()
                .filter_map(|(k, v)| parse_age(k.value(), v.value()))
                .collect(),
        )
    }

    fn remove_keys(&self, keys: &[String]) {
        let Ok(mut tx) = self.db.begin_write() else {
            return;
        };
        if tx.set_durability(Durability::None).is_err() {
            return;
        }
        let Ok(mut table) = tx.open_table(ENTRIES) else {
            return;
        };
        for key in keys {
            let _ = table.remove(key.as_str());
        }
        drop(table);
        drop(tx.commit());
    }
}

/// Parse one stored row into `(timestamp, key)`; corrupt rows are skipped
/// by the caller so one bad entry cannot break pruning.
fn parse_age(key: &str, payload: &[u8]) -> Option<(u64, String)> {
    let f: CachedFile = serde_json::from_slice(payload).ok()?;
    Some((f.ts, key.to_string()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Remove the pre-redb one-JSON-per-entry files left behind in the cache
/// directory. Best effort: leftovers only waste disk space.
fn drop_legacy_entries(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for e in entries.flatten() {
        let is_json = e.path().extension().is_some_and(|x| x == "json");
        if is_json {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("abcop-cache-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Write one raw JSON payload directly through the table.
    fn seed_raw(cache: &Cache, key: &str, payload: &[u8]) {
        let mut tx = cache.db.begin_write().unwrap();
        tx.set_durability(Durability::None).unwrap();
        let mut table = tx.open_table(ENTRIES).unwrap();
        table.insert(key, payload).unwrap();
        drop(table);
        tx.commit().unwrap();
    }

    /// Insert `count` well-formed but empty entries with distinct
    /// ascending timestamps (`0000…` .. count-1).
    fn seed_entries(cache: &Cache, count: u64) {
        for i in 0..count {
            let payload =
                format!(r#"{{"ts":{i},"abc":[],"used_once":[],"never_used":[],"oversize":null}}"#);
            seed_raw(cache, &format!("{i:064}"), payload.as_bytes());
        }
    }

    fn sample(oversize: Option<usize>) -> (Vec<crate::abc::AbcOffense>, CachedDiags) {
        let mk = || {
            vec![crate::abc::AbcOffense {
                line: 7,
                end_line: 19,
                column: 3,
                name: "Foo#bar".into(),
                score: 22.5,
                vector: "<9, 4, 2>".into(),
            }]
        };
        let abc = mk();
        let diags = (
            mk(),
            vec![crate::used_once::UsedOnceOffense {
                line: 12,
                column: 4,
                name: "x".into(),
            }],
            vec![crate::never_used::NeverUsedOffense {
                line: 30,
                column: 2,
                name: "dead".into(),
            }],
            oversize,
        );
        (abc, diags)
    }

    #[test]
    fn roundtrip_returns_stored_diagnostics() {
        let dir = temp_cache_dir("roundtrip");
        let cache = Cache::open_at(&dir).expect("cache opens");
        let (_, diags) = sample(Some(210));
        let key = "a".repeat(64);
        cache.store(&key, &diags.0, &diags.1, &diags.2, diags.3);

        let hit = cache.get(&key).expect("cache hit");
        assert_eq!(hit.0[0].name, "Foo#bar");
        assert_eq!(hit.0[0].score, 22.5);
        assert_eq!(hit.1[0].name, "x");
        assert_eq!(hit.2[0].name, "dead");
        assert_eq!(hit.3, Some(210));
    }

    #[test]
    fn miss_on_other_key_is_none() {
        let dir = temp_cache_dir("miss");
        let cache = Cache::open_at(&dir).expect("cache opens");
        let (_, diags) = sample(Some(210));
        cache.store(&"a".repeat(64), &diags.0, &diags.1, &diags.2, diags.3);
        // note: store signature is (&[u8] key, &abc, &used, &never, oversize)
        assert!(cache.get(&"b".repeat(64)).is_none(), "unrelated key misses");
    }

    #[test]
    fn prune_keeps_newest_max_entries() {
        let dir = temp_cache_dir("prune");
        let cache = Cache::open_at(&dir).expect("cache opens");
        seed_entries(&cache, MAX_ENTRIES as u64 + 10);
        cache.prune();

        assert_eq!(count_rows(&cache), MAX_ENTRIES);
        // oldest ten dropped, newest kept
        assert!(
            (0..=9).all(|i| entry_absent(&cache, i)),
            "oldest ten pruned"
        );
        assert!(entry_present(&cache, MAX_ENTRIES as u64 + 9));
    }

    fn row_exists(cache: &Cache, i: u64) -> bool {
        let rtx = cache.db.begin_read().unwrap();
        let table = rtx.open_table(ENTRIES).unwrap();
        table.get(format!("{i:064}").as_str()).unwrap().is_some()
    }

    fn entry_absent(cache: &Cache, i: u64) -> bool {
        !row_exists(cache, i)
    }

    fn entry_present(cache: &Cache, i: u64) -> bool {
        row_exists(cache, i)
    }

    fn count_rows(cache: &Cache) -> usize {
        let rtx = cache.db.begin_read().unwrap();
        let table = rtx.open_table(ENTRIES).unwrap();
        table.len().unwrap() as usize
    }

    #[test]
    fn corrupt_entry_is_a_miss_not_a_crash() {
        let dir = temp_cache_dir("corrupt");
        let cache = Cache::open_at(&dir).expect("cache opens");
        let key = "c".repeat(64);
        seed_raw(&cache, &key, b"{not json");
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn legacy_json_files_are_removed_on_open() {
        let dir = temp_cache_dir("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("deadbeef.json"), b"{}").unwrap();
        std::fs::write(dir.join("keepme.txt"), b"x").unwrap();
        Cache::open_at(&dir).expect("cache opens");
        assert!(!dir.join("deadbeef.json").exists(), "legacy entry removed");
        assert!(dir.join("keepme.txt").exists(), "non-json untouched");
    }
}

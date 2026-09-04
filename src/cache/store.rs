//! Storage core of the result cache: the single redb table, the JSON
//! entry codec, and the recency-retention policy that bounds the table
//! to [`MAX_ENTRIES`] newest rows.
//!
//! Ownership here is deliberately narrow: rows in, rows out, oldest out
//! when the budget overflows. Bootstrap (where the database file lives,
//! disabled mode, legacy cleanup) stays in the [`super`](super) facade;
//! diagnostics identity (what goes into a key) belongs to it too.

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use redb::ReadableTableMetadata;
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};

/// Bump whenever counting rules or output shape change so stale entries are
/// never served.
pub(crate) const RULES_REV: u32 = 31;

pub(crate) const MAX_ENTRIES: usize = 20_000;
const ENTRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entries");

pub(crate) struct EntryStore {
    pub(super) db: Database,
}

impl EntryStore {
    pub(crate) fn new(db: Database) -> Self {
        Self { db }
    }

    pub(crate) fn get(&self, key: &str) -> Option<CachedDiags> {
        let rtx = self.db.begin_read().ok()?;
        let table = rtx.open_table(ENTRIES).ok()?;
        let value = table.get(key).ok()??;
        let f: CachedFile = serde_json::from_slice(value.value()).ok()?;
        Some((f.abc, f.used_once, f.never_used, f.module_abc))
    }

    pub(crate) fn store(
        &self,
        key: &str,
        abc: &[crate::abc::AbcOffense],
        used_once: &[crate::used_once::UsedOnceOffense],
        never_used: &[crate::never_used::NeverUsedOffense],
        module_abc: Option<crate::modulesize::ModuleAbc>,
    ) {
        let payload = CachedFileRef {
            ts: now_ms(),
            abc,
            used_once,
            never_used,
            module_abc,
        };
        let bytes = serde_json::to_vec(&payload);
        let tx = self.write_tx();
        let Ok(bytes) = bytes else { return };
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
    pub(crate) fn prune(&self) {
        let Some(by_age) = self.entries_by_age() else {
            return;
        };
        if by_age.len() <= MAX_ENTRIES {
            return;
        }
        let mut newest_first = by_age;
        newest_first.sort_by_key(|(t, _)| std::cmp::Reverse(*t));

        self.remove_keys(
            &newest_first
            .iter()
            .skip(MAX_ENTRIES)
            .map(|(_, k)| k.clone())
                .collect::<Vec<_>>(),
        );
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
        let tx = self.write_tx();
        let Ok(mut table) = tx.open_table(ENTRIES) else {
            return;
        };
        for key in keys {
            let _ = table.remove(key.as_str());
        }
        drop(table);
        drop(tx.commit());
    }

    /// Writable transaction with the relaxed durability the cache's
    /// correctness model allows (`Durability::None`).
    pub(super) fn write_tx(&self) -> redb::WriteTransaction {
        let mut tx = self
            .db
            .begin_write()
            .expect("fresh write transaction on cache db");
        tx.set_durability(Durability::None)
            .expect("relaxed durability accepted");
        tx
    }

    /// Raw row lookup for tests: the store is the unit that owns layout.
    #[cfg(test)]
    pub(super) fn raw_get(&self, key: &str) -> Option<usize> {
        self.db
            .begin_read()
            .unwrap()
            .open_table(ENTRIES)
            .unwrap()
            .get(key)
            .unwrap()
            .map(|_| 1_usize)
    }

    /// Row count for tests.
    #[cfg(test)]
    pub(super) fn raw_len(&self) -> usize {
        self.db
            .begin_read()
            .unwrap()
            .open_table(ENTRIES)
            .unwrap()
            .len()
            .unwrap() as usize
    }

    /// Direct insert bypassing the codec, for tests seeding malformed or
    /// hand-built rows.
    #[cfg(test)]
    pub(super) fn raw_insert(&self, key: &str, payload: &[u8]) {
        let tx = self.write_tx();
        let mut table = tx.open_table(ENTRIES).unwrap();
        table.insert(key, payload).unwrap();
        drop(table);
        tx.commit().unwrap();
    }
}

#[derive(serde::Deserialize)]
struct CachedFile {
    ts: u64,
    abc: Vec<crate::abc::AbcOffense>,
    used_once: Vec<crate::used_once::UsedOnceOffense>,
    never_used: Vec<crate::never_used::NeverUsedOffense>,
    module_abc: Option<crate::modulesize::ModuleAbc>,
}

/// Borrowing twin of [`CachedFile`] for serialization straight from the
/// analysis result -- no Clone required on the offense types.
#[derive(serde::Serialize)]
struct CachedFileRef<'a> {
    ts: u64,
    abc: &'a [crate::abc::AbcOffense],
    used_once: &'a [crate::used_once::UsedOnceOffense],
    never_used: &'a [crate::never_used::NeverUsedOffense],
    module_abc: Option<crate::modulesize::ModuleAbc>,
}

/// The stored-diagnostics tuple handed back to the pipeline.
pub type CachedDiags = (
    Vec<crate::abc::AbcOffense>,
    Vec<crate::used_once::UsedOnceOffense>,
    Vec<crate::never_used::NeverUsedOffense>,
    Option<crate::modulesize::ModuleAbc>,
);

/// Parse one stored row into `(timestamp, key)`; corrupt rows are skipped
/// by the caller so one bad entry cannot break pruning.
fn parse_age(key: &str, payload: &[u8]) -> Option<(u64, String)> {
    serde_json::from_slice::<CachedFile>(payload)
        .ok()
        .map(|f| (f.ts, key.to_string()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

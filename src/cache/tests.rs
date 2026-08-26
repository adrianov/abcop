//! Cache tests spanning both units: facade bootstrap (legacy cleanup)
//! and the entry store (codec roundtrip, key isolation, retention).

use super::*;

fn temp_cache_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("abcop-cache-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
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
    assert!(cache.get(&"b".repeat(64)).is_none(), "unrelated key misses");
}

#[test]
fn prune_keeps_newest_max_entries() {
    use crate::cache::store::MAX_ENTRIES;

    fn seed_entries(cache: &Cache, count: u64) {
        for i in 0..count {
            let payload =
                format!(r#"{{"ts":{i},"abc":[],"used_once":[],"never_used":[],"oversize":null}}"#);
            cache
                .store_ref()
                .raw_insert(&format!("{i:064}"), payload.as_bytes());
        }
    }
    let dir = temp_cache_dir("prune");
    let cache = Cache::open_at(&dir).expect("cache opens");
    seed_entries(&cache, MAX_ENTRIES as u64 + 10);
    cache.prune();

    assert_eq!(cache.store_ref().raw_len(), MAX_ENTRIES);
    // oldest ten dropped, newest kept
    assert!(
        (0..=9).all(|i| cache.store_ref().raw_get(&format!("{i:064}")).is_none()),
        "oldest ten pruned"
    );
    let newest = format!("{:064}", MAX_ENTRIES as u64 + 9);
    assert!(cache.store_ref().raw_get(&newest).is_some());
}

#[test]
fn corrupt_entry_is_a_miss_not_a_crash() {
    let dir = temp_cache_dir("corrupt");
    let cache = Cache::open_at(&dir).expect("cache opens");
    let key = "c".repeat(64);
    cache.store_ref().raw_insert(&key, b"{not json");
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

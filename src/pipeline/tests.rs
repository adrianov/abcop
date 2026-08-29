//! End-to-end pipeline contracts: cache round-trips stay scope-narrowed,
//! and the ModuleAbcSize refactor-scale rule behaves identically for prod
//! and spec trees.

use super::analyze_one;
use super::narrow::apply;
use crate::git_changes::{Changeset, Lines};
use crate::modulesize::ModuleAbc;
use crate::output::FileResult;
use std::collections::{BTreeMap, BTreeSet};

fn cs_with(rel: &str, lines: Lines) -> Changeset {
    Changeset {
        root: "/repo".into(),
        files: BTreeMap::from([(rel.into(), lines)]),
    }
}

fn cs_with_ranges(rel: &str, lo: usize, hi: usize) -> Changeset {
    cs_with(rel, Lines::Ranges((lo..=hi).collect()))
}

fn sample_module_abc() -> ModuleAbc {
    ModuleAbc {
        score: 120.0,
        vector: "<40, 100, 40>".into(),
    }
}

fn result_with_module_abc() -> FileResult {
    FileResult {
        path: "/repo/app/models/big.rb".into(),
        abc: Vec::new(),
        used_once: Vec::new(),
        never_used: Vec::new(),
        module_abc: Some(sample_module_abc()),
    }
}

#[test]
fn small_diffs_into_large_modules_are_not_flagged() {
    let cs = cs_with_ranges("app/models/big.rb", 3, 9);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, b"");
    assert_eq!(r.module_abc, None);
}

#[test]
fn refactor_scale_diffs_keep_module_abc() {
    let set: BTreeSet<usize> = (1..=120).collect();
    let cs = cs_with("app/models/big.rb", Lines::Ranges(set));
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, b"");
    assert_eq!(r.module_abc, Some(sample_module_abc()));
}

#[test]
fn new_files_count_as_fully_changed() {
    let cs = cs_with("app/models/big.rb", Lines::All);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, b"");
    assert_eq!(r.module_abc, Some(sample_module_abc()));
}

#[test]
fn full_scans_keep_module_abc() {
    let mut r = result_with_module_abc();
    apply(None, &mut r, b"");
    assert_eq!(r.module_abc, Some(sample_module_abc()));
}

/// A 60-function Ruby fixture under `dir/rel` with `template` bodies.
fn filler_ruby_file(
    dir: &std::path::Path,
    rel: &str,
    template: &str,
) -> (std::path::PathBuf, String) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join(rel).parent().unwrap()).unwrap();
    let file = dir.join(rel);
    let mut src = String::new();
    for i in 1..=60 {
        src.push_str(&template.replace("{i}", &i.to_string()));
    }
    std::fs::write(&file, &src).unwrap();
    (file, src)
}

fn scoped_changeset(root: &std::path::Path, rel: &str, hi: usize) -> Changeset {
    Changeset {
        root: root.display().to_string(),
        files: BTreeMap::from([(rel.to_string(), Lines::Ranges((1..=hi).collect()))]),
    }
}

/// Regression: a warm-cache hit used to skip narrowing, so scoped runs
/// served un-narrowed results. Both paths must narrow identically.
/// A narrowed-scope fixture: temp repo root, the filler file inside it,
/// and a 3-line changeset over that file.
fn narrow_fixture() -> (std::path::PathBuf, std::path::PathBuf, Changeset) {
    let dir = std::env::temp_dir().join(format!("abcop_pipeline_narrow_{}", std::process::id()));
    let rel = "app/models_big.rb";
    let (file, _) = filler_ruby_file(
        &dir,
        rel,
        "def filler_{i}\n  x_{i} = 1\n  unused_{i} = 2\nend\n\n",
    );
    let cs = scoped_changeset(&dir, rel, 2);
    (dir, file, cs)
}

#[test]
fn cache_hits_are_scope_narrowed_like_fresh_analysis() {
    let (dir, file, cs) = narrow_fixture();

    let first = assert_fresh_unflagged(&file, &cs);

    // Warm cache path over a run-wide cache directory.
    let cache = crate::cache::Cache::open_at(&dir.join(".cache")).expect("cache");
    let _ = analyze_one(&file, None, 17.0, Some(&cs), Some(&cache));
    let second = analyze_one(&file, None, 17.0, Some(&cs), Some(&cache));

    assert_same_diagnostics(&first, &second);
    assert_eq!(second.module_abc, None, "cache hit must stay narrowed");

    let _ = std::fs::remove_dir_all(&dir);
}

fn assert_same_diagnostics(a: &FileResult, b: &FileResult) {
    assert_eq!(a.abc, b.abc);
    assert_eq!(a.used_once, b.used_once);
    assert_eq!(a.never_used, b.never_used);
}
/// Fresh analysis path for the fixture: a 3-line diff must not flag size.
fn assert_fresh_unflagged(file: &std::path::Path, cs: &Changeset) -> FileResult {
    let r = analyze_one(file, None, 17.0, Some(cs), None);
    assert_eq!(r.module_abc, None, "3-line diff must not flag size");
    r
}

/// A 60-test spec fixture plus its repo root.
fn spec_fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!(
        "abcop_pipeline_specsize_{}_{}",
        std::process::id(),
        tag
    ));
    let (file, src) = filler_ruby_file(
        &dir,
        "test/commit_plan_finalize_test.rb",
        "def test_fill_{i}\n  x = 1\n  y = 2\n  assert true\nend\n\n",
    );
    (dir, file, src)
}

#[test]
fn small_spec_diff_keeps_the_test_tree_exemption() {
    let (dir, file, src) = spec_fixture("small");
    let cs = scoped_changeset(&dir, "test/commit_plan_finalize_test.rb", 3);
    let mut r = analyze_one(&file, None, 17.0, Some(&cs), None);
    apply(Some(&cs), &mut r, src.as_bytes());
    assert_eq!(r.module_abc, None, "small diff keeps spec-tree exemption");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refactor_scale_spec_diff_is_size_accountable() {
    let (dir, file, src) = spec_fixture("big");
    let cs = scoped_changeset(&dir, "test/commit_plan_finalize_test.rb", 120);
    let mut r = analyze_one(&file, None, 17.0, Some(&cs), None);
    apply(Some(&cs), &mut r, src.as_bytes());
    assert!(
        r.module_abc.as_ref().is_some_and(|m| m.score > 90.0),
        "refactor-scale spec diff must surface ModuleAbcSize, got {:?}",
        r.module_abc
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn full_scan_drops_module_abc_on_test_trees() {
    let (dir, file, _) = spec_fixture("full");
    let r = analyze_one(&file, None, 17.0, None, None);
    assert_eq!(r.module_abc, None, "full scans keep the test-tree exemption");
    let _ = std::fs::remove_dir_all(&dir);
}

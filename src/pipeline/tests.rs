//! End-to-end pipeline contracts: cache round-trips stay scope-narrowed,
//! AbcSize reports changed over-threshold methods, and ModuleAbcSize
//! re-scores from changed methods only.

use super::analyze_one;
use super::narrow::apply;
use crate::abc::{AbcOffense, Limits};
use crate::git_changes::{Changeset, Lines};
use crate::modulesize::{self, ModuleAbc};
use crate::output::FileResult;
use std::collections::BTreeMap;

fn test_limits() -> Limits {
    Limits {
        method: 17.0,
        module: modulesize::MAX_ABC,
    }
}

fn cs_with(rel: &str, lines: Lines) -> Changeset {
    Changeset {
        root: "/repo".into(),
        files: BTreeMap::from([(rel.into(), lines)]),
    }
}

fn cs_with_ranges(rel: &str, lo: usize, hi: usize) -> Changeset {
    cs_with(rel, Lines::Ranges((lo..=hi).collect()))
}

fn method(line: usize, end_line: usize, a: u32, b: u32, c: u32) -> AbcOffense {
    AbcOffense {
        name: "m".into(),
        line,
        end_line,
        column: 0,
        score: (((a * a + b * b + c * c) as f64).sqrt() * 100.0).round() / 100.0,
        vector: format!("<{a}, {b}, {c}>"),
    }
}

fn sample_abc() -> AbcOffense {
    method(5, 20, 10, 40, 20)
}

/// Oversized module: three dense methods; only the middle one sits in a
/// typical small-diff window.
fn sample_module_abc() -> ModuleAbc {
    let methods = vec![
        method(1, 40, 40, 40, 0),
        method(50, 90, 40, 40, 0),
        method(100, 140, 40, 40, 0),
    ];
    let (a, b, c, score) = crate::abc::module_score(&methods);
    ModuleAbc {
        score,
        vector: crate::abc::fmt_vector(a, b, c),
        methods,
    }
}

fn result_with_size_findings(path: &str) -> FileResult {
    FileResult {
        path: path.into(),
        abc: vec![sample_abc()],
        used_once: Vec::new(),
        never_used: Vec::new(),
        module_abc: Some(sample_module_abc()),
    }
}

fn result_with_module_abc() -> FileResult {
    result_with_size_findings("/repo/app/models/big.rb")
}

#[test]
fn small_diffs_still_report_method_abc() {
    let cs = cs_with_ranges("app/models/big.rb", 3, 9);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, modulesize::MAX_ABC, b"");
    assert_eq!(
        r.abc,
        vec![sample_abc()],
        "AbcSize keeps intersecting methods"
    );
}

#[test]
fn small_diffs_rescope_module_under_ceiling() {
    let cs = cs_with_ranges("app/models/big.rb", 50, 55);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, modulesize::MAX_ABC, b"");
    assert_eq!(
        r.module_abc, None,
        "one medium method must not keep full-module ModuleAbcSize"
    );
}

#[test]
fn changed_methods_over_ceiling_keep_module_abc() {
    let cs = cs_with_ranges("app/models/big.rb", 1, 140);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, modulesize::MAX_ABC, b"");
    let hit = r.module_abc.expect("all methods changed => still over");
    assert!(hit.score > modulesize::MAX_ABC);
    assert_eq!(hit.methods.len(), 3);
}

#[test]
fn new_files_count_as_fully_changed() {
    let cs = cs_with("app/models/big.rb", Lines::All);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r, modulesize::MAX_ABC, b"");
    assert_eq!(
        r.module_abc.as_ref().map(|m| m.score),
        Some(sample_module_abc().score)
    );
}

#[test]
fn full_scans_keep_module_abc() {
    let mut r = result_with_module_abc();
    apply(None, &mut r, modulesize::MAX_ABC, b"");
    assert_eq!(
        r.module_abc.as_ref().map(|m| m.score),
        Some(sample_module_abc().score)
    );
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
    let second = warm_narrowed(&file, &cs, &dir.join(".cache"));
    assert_same_diagnostics(&first, &second);
    assert_eq!(second.module_abc, None, "cache hit must stay narrowed");
    let _ = std::fs::remove_dir_all(&dir);
}

fn warm_narrowed(
    file: &std::path::Path,
    cs: &Changeset,
    cache_dir: &std::path::Path,
) -> FileResult {
    let cache = crate::cache::Cache::open_at(cache_dir).expect("cache");
    let _ = analyze_one(file, None, test_limits(), Some(cs), Some(&cache));
    analyze_one(file, None, test_limits(), Some(cs), Some(&cache))
}

fn assert_same_diagnostics(a: &FileResult, b: &FileResult) {
    assert_eq!(a.abc, b.abc);
    assert_eq!(a.used_once, b.used_once);
    assert_eq!(a.never_used, b.never_used);
}

/// Fresh analysis path for the fixture: a 3-line diff must not flag size.
fn assert_fresh_unflagged(file: &std::path::Path, cs: &Changeset) -> FileResult {
    let r = analyze_one(file, None, test_limits(), Some(cs), None);
    assert_eq!(r.module_abc, None, "3-line diff must not flag size");
    r
}

/// Dense method bodies so many changed methods sum past ModuleAbcSize.
fn dense_ruby_template() -> &'static str {
    "def fill_{i}\n  a = 1\n  b = 2\n  c = 3\n  d = 4\n  e = 5\n  \
     f = 6\n  g = 7\n  h = 8\n  i = 9\n  j = 10\n  \
     return a + b + c + d + e + f + g + h + i + j if a > 0 && b > 0\n  \
     foo(a, b, c, d, e, f, g, h, i, j)\nend\n\n"
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
        dense_ruby_template(),
    );
    (dir, file, src)
}

#[test]
fn small_spec_diff_keeps_module_quiet() {
    let (dir, file, src) = spec_fixture("small");
    let cs = scoped_changeset(&dir, "test/commit_plan_finalize_test.rb", 3);
    let mut r = analyze_one(&file, None, test_limits(), Some(&cs), None);
    apply(Some(&cs), &mut r, modulesize::MAX_ABC, src.as_bytes());
    assert_eq!(r.module_abc, None, "few changed methods stay under ceiling");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn many_changed_spec_methods_are_size_accountable() {
    let (dir, file, src) = spec_fixture("big");
    let cs = scoped_changeset(&dir, "test/commit_plan_finalize_test.rb", 2000);
    let mut r = analyze_one(&file, None, test_limits(), Some(&cs), None);
    apply(Some(&cs), &mut r, modulesize::MAX_ABC, src.as_bytes());
    assert!(
        r.module_abc
            .as_ref()
            .is_some_and(|m| m.score > modulesize::MAX_ABC),
        "changed-method sum over ceiling must surface ModuleAbcSize, got {:?}",
        r.module_abc
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn full_scan_drops_module_abc_on_test_trees() {
    let (dir, file, _) = spec_fixture("full");
    let r = analyze_one(&file, None, test_limits(), None, None);
    assert_eq!(
        r.module_abc, None,
        "full scans keep the test-tree exemption"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

//! Synthetic pipeline contracts: AbcSize keeps changed methods, and
//! ModuleAbcSize re-scores from changed methods only.

use super::narrow::apply;
use crate::abc::AbcOffense;
use crate::git_changes::{Changeset, Lines};
use crate::modulesize::{self, ModuleAbc};
use crate::output::FileResult;
use std::collections::BTreeMap;

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

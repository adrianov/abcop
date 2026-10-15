//! Synthetic pipeline contracts: AbcSize keeps changed methods.
//! ModuleAbcSize is a whole-file score, so a diff does not rescore it.

use super::narrow::apply;
use crate::abc::AbcOffense;
use crate::git_changes::{Changeset, Lines};
use crate::modulesize::ModuleAbc;
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
        start_byte: 0,
        end_byte: 0,
        prior: None,
    }
}

fn sample_abc() -> AbcOffense {
    method(5, 20, 10, 40, 20)
}

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
        prior: None,
    }
}

fn result_with_size_findings(path: &str) -> FileResult {
    FileResult {
        path: path.into(),
        abc: vec![sample_abc()],
        used_once: Vec::new(),
        never_used: Vec::new(),
        module_abc: Some(sample_module_abc()),
        module_score: sample_module_abc().score,
        units: Vec::new(),
    }
}

fn result_with_module_abc() -> FileResult {
    result_with_size_findings("/repo/app/models/big.rb")
}

#[test]
fn small_diffs_still_report_method_abc() {
    let cs = cs_with_ranges("app/models/big.rb", 3, 9);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r);
    assert_eq!(
        r.abc,
        vec![sample_abc()],
        "AbcSize keeps intersecting methods"
    );
}

#[test]
fn small_diffs_keep_the_whole_module_score() {
    let cs = cs_with_ranges("app/models/big.rb", 50, 55);
    let mut r = result_with_module_abc();
    apply(Some(&cs), &mut r);
    assert_eq!(
        r.module_abc.as_ref().map(|m| m.score),
        Some(sample_module_abc().score),
        "a small diff must not rescore the module from touched methods"
    );
}

#[test]
fn full_scans_keep_module_abc() {
    let mut r = result_with_module_abc();
    apply(None, &mut r);
    assert_eq!(
        r.module_abc.as_ref().map(|m| m.score),
        Some(sample_module_abc().score)
    );
}

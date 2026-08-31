//! Metrics/ModuleAbcSize: production modules must stay under MAX_ABC.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt.
//! Scoped runs re-score from methods that intersect the diff only.

mod classify;
#[cfg(test)]
mod tests;

pub(crate) use classify::{
    GENERATED_DIR_PAIRS, GENERATED_DIRS, is_generated_name, is_route_table, is_third_party,
};

use crate::abc::{self, AbcOffense};

/// Default ModuleAbcSize ceiling. Calibrated against ~200-line Ruby modules
/// in RuboCop/Sinatra lib trees (~85–90 median); 120 is a looser default
/// so typical mid-size files stay quiet while still flagging oversized ones.
pub const MAX_ABC: f64 = 120.0;

pub(crate) const NON_PROD_DIRS: [&str; 9] = [
    "spec/",
    "specs/",
    "test/",
    "tests/",
    "__tests__/",
    "__mocks__/",
    "testdata/",
    "testing/",
    "fixtures/",
];

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModuleAbc {
    pub score: f64,
    pub vector: String,
    /// Method ABC parts that formed this total. Scoped runs re-sum the
    /// subset intersecting the diff against the same ceiling.
    #[serde(default)]
    pub(crate) methods: Vec<AbcOffense>,
}

pub(crate) fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    in_non_prod_dir(&p) || is_test_basename(base_name(&p))
}

fn in_non_prod_dir(p: &str) -> bool {
    NON_PROD_DIRS.iter().any(|d| p.contains(d))
}

fn base_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn is_test_basename(base: &str) -> bool {
    base.starts_with("test_")
        || base.starts_with("spec.")
        || base.starts_with("test.")
        || base == "conftest.py"
        || base.contains("_spec.")
        || base.contains("_test.")
        || base.contains(".tests.")
        || base.ends_with(".spec.")
        || base.ends_with(".test.")
}

pub fn is_production(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let technical = p.ends_with("schema.rb") || p.ends_with(".lock") || p.contains("schema.rb");
    !is_test_path(path) && !technical
}

/// File ABC over method scores; Rust `#[cfg(test)]` tails are omitted
/// (same exemption the old line budget used).
pub fn from_scores(
    scores: &[AbcOffense],
    path: &str,
    src: &str,
    max: f64,
) -> Option<ModuleAbc> {
    let filtered = rust_prod_scores(scores, path, src);
    scored(filtered.into_owned(), max)
}

/// Re-sum `module_abc` from methods that `keep` accepts; clear when the
/// rescored total no longer exceeds `max`.
pub(crate) fn rescope(
    module_abc: &mut Option<ModuleAbc>,
    max: f64,
    keep: impl Fn(&AbcOffense) -> bool,
) {
    let Some(m) = module_abc.take() else {
        return;
    };
    let methods: Vec<_> = m.methods.into_iter().filter(|o| keep(o)).collect();
    *module_abc = scored(methods, max);
}

fn scored(methods: Vec<AbcOffense>, max: f64) -> Option<ModuleAbc> {
    let (a, b, c, score) = abc::module_score(&methods);
    (score > max).then(|| ModuleAbc {
        score,
        vector: abc::fmt_vector(a, b, c),
        methods,
    })
}

/// Drop non-production ModuleAbcSize hits on full (unscoped) scans.
pub(crate) fn drop_non_production(path: &str, module_abc: &mut Option<ModuleAbc>) {
    if module_abc.is_some() && !is_production(path) {
        *module_abc = None;
    }
}

fn rust_prod_scores<'a>(
    scores: &'a [AbcOffense],
    path: &str,
    src: &str,
) -> std::borrow::Cow<'a, [AbcOffense]> {
    if !path.ends_with(".rs") {
        return std::borrow::Cow::Borrowed(scores);
    }
    let Some(pos) = src
        .lines()
        .position(|l| l.trim_start().starts_with("#[cfg(test)]"))
    else {
        return std::borrow::Cow::Borrowed(scores);
    };
    let cutoff = pos + 1;
    std::borrow::Cow::Owned(
        scores
            .iter()
            .filter(|o| o.line < cutoff)
            .cloned()
            .collect(),
    )
}

//! Metrics/ModuleAbcSize: production modules must stay under MAX_ABC.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt.
//! Scoped runs also share a refactor-scale size gate (see [`SizeGate`]).

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

/// In scoped runs, AbcSize and ModuleAbcSize only fire when the diff itself
/// touches at least this many lines of the file (when [`SizeGate`] covers
/// that path): a refactor-scale change invites the size conversation, while
/// a three-line patch into an already-large legacy module should not.
pub(crate) const MIN_REVIEW_REFACTOR_LINES: usize = 100;

/// Which paths the scoped ≥100-line size gate covers for AbcSize and
/// ModuleAbcSize. Small edits into oversized legacy files stay silent when
/// the gate applies; UsedOnce / NeverUsed are unaffected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum SizeGate {
    /// Gate only test/spec trees; production size findings always report.
    Specs,
    /// Never suppress by line count; report every intersecting size finding.
    #[value(name = "none")]
    Never,
    /// Gate production and test paths (default for foreign / legacy trees).
    #[default]
    Both,
}

impl SizeGate {
    /// True when this gate's ≥100-line threshold applies to `path`.
    pub(crate) fn covers(self, path: &str) -> bool {
        match self {
            Self::Never => false,
            Self::Both => true,
            Self::Specs => is_test_path(path),
        }
    }
}

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
    let (a, b, c, score) = abc::module_score(filtered.as_ref());
    (score > max).then(|| ModuleAbc {
        score,
        vector: abc::fmt_vector(a, b, c),
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

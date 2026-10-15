//! Metrics/ModuleAbcSize: a module's Fitzpatrick total of its function vectors.
//! New modules must stay under [`MAX_ABC`]. An existing module may stay over
//! that ceiling when its score does not rise against `HEAD`.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt on a
//! whole-tree scan.

mod classify;
#[cfg(test)]
mod tests;

pub(crate) use classify::{
    GENERATED_DIR_PAIRS, GENERATED_DIRS, is_fixture_tree, is_generated_name, is_route_table,
    is_third_party,
};

use crate::abc::{self, AbcOffense};

/// Default ModuleAbcSize ceiling. Calibrated against ~200-line Ruby modules
/// in RuboCop/Sinatra lib trees (~85–90 median); 120 is a looser default
/// so typical mid-size files stay quiet while still flagging oversized ones.
/// A normal scan applies it to modules absent from `HEAD`; an existing
/// module is reported only when its score rose.
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
    /// HEAD score when this hit is a rise. Absent for a new module and
    /// for `--full` / `--everything`, which report every over-ceiling module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<f64>,
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

/// Whole-module magnitude, then a hit when it exceeds `max`.
/// Rust `#[cfg(test)]` tails are omitted (same exemption the old line budget used).
pub(crate) fn measure(
    scores: &[AbcOffense],
    path: &str,
    src: &str,
    max: f64,
) -> (f64, Option<ModuleAbc>) {
    let (a, b, c, score) = abc::module_score(&rust_prod_scores(scores, path, src));
    (
        score,
        (score > max).then(|| ModuleAbc {
            score,
            vector: abc::fmt_vector(a, b, c),
            prior: None,
        }),
    )
}

/// Lead sentence of a ModuleAbcSize finding. `show_budget` adds the `/max`
/// budget used by text output; JSON and MCP omit it. The return target
/// is [`abc::rise_clause`].
pub(crate) fn finding_message(m: &ModuleAbc, ceiling: f64, show_budget: bool) -> String {
    let lead = abc::rise_clause(m.prior, m.score, ceiling);
    let numbers = if show_budget {
        format!("{} {}/{}", m.vector, abc::g4(m.score), abc::g4(ceiling))
    } else {
        format!("{} {}", m.vector, abc::g4(m.score))
    };
    let suffix = if m.prior.is_some() {
        ""
    } else {
        " -- extract a coherent subunit"
    };
    format!("Assignment Branch Condition size for module {lead}. [{numbers}]{suffix}")
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
    std::borrow::Cow::Owned(scores.iter().filter(|o| o.line < cutoff).cloned().collect())
}

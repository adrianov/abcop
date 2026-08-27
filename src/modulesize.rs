//! ModuleSize rule: production modules must stay under MAX_LINES.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt.

mod classify;
#[cfg(test)]
mod tests;

pub(crate) use classify::{
    GENERATED_DIR_PAIRS, GENERATED_DIRS, is_generated_name, is_route_table, is_third_party,
};

pub const MAX_LINES: usize = 200;

/// In --mr/--changed scopes ModuleSize only fires when the diff itself
/// touches at least this many lines: a refactor-scale change invites the
/// size conversation, while a three-line patch into an already-large
/// legacy module should not gate the review.
pub(crate) const MIN_REVIEW_REFACTOR_LINES: usize = 100;

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

/// Effective size: for Rust sources the trailing `#[cfg(test)]` module does
/// not count toward the budget (mirrors the spec-file exemption).
pub fn effective_lines(src: &str, path: &str) -> usize {
    let mut n = src.lines().count();
    if path.ends_with(".rs")
        && let Some(pos) = src
            .lines()
            .position(|l| l.trim_start().starts_with("#[cfg(test)]"))
    {
        n = pos;
    }
    n
}

pub fn is_production(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let technical = p.ends_with("schema.rb") || p.ends_with(".lock") || p.contains("schema.rb");
    !is_test_path(path) && !technical
}
/// Returns Some(lines) when the module exceeds the budget.
/// Data files that no supported language parses (Qt Linguist `.ts` XML,
/// and XML generally): line counts on them are meaningless noise.
fn is_data_file(src: &str) -> bool {
    src.trim_start().starts_with("<?xml")
}

pub fn offense(src: &str, path: &str) -> Option<usize> {
    if !is_production(path) || is_data_file(src) {
        return None;
    }
    let lines = effective_lines(src, path);
    (lines >= MAX_LINES).then_some(lines)
}

//! ModuleSize rule: production modules must stay under MAX_LINES.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt.

pub const MAX_LINES: usize = 200;

fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    const DIRS: [&str; 9] = [
        "spec/", "specs/", "test/", "tests/", "__tests__/", "__mocks__/",
        "testdata/", "testing/", "fixtures/",
    ];
    if DIRS.iter().any(|d| p.contains(d)) {
        return true;
    }
    let base = p.rsplit('/').next().unwrap_or(p.as_str());
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
    let technical = p.ends_with("schema.rb")
        || p.ends_with(".lock")
        || p.contains("schema.rb");
    !is_test_path(path) && !technical
}

/// Returns Some(lines) when the module exceeds the budget.
pub fn offense(src: &str, path: &str) -> Option<usize> {
    if !is_production(path) {
        return None;
    }
    let lines = effective_lines(src, path);
    (lines >= MAX_LINES).then_some(lines)
}

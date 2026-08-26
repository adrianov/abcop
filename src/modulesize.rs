//! ModuleSize rule: production modules must stay under MAX_LINES.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt.

pub const MAX_LINES: usize = 200;

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

/// Vendored dependencies, package caches and build outputs: third-party or
/// machine-generated trees are never the review target of a default scan.
/// Walker-side only (see `skip::skipped_by_default`); ModuleSize's own
/// classification above stays unchanged.
pub(crate) const GENERATED_DIRS: [&str; 29] = [
    "vendor/",
    ".bundle/",
    "node_modules/",
    "bower_components/",
    "Pods/",
    "Carthage/",
    "target/",
    "dist/",
    "build/",
    "out/",
    ".next/",
    ".nuxt/",
    "_build/",
    "deps/",
    "coverage/",
    "venv/",
    ".venv/",
    "site-packages/",
    "__pycache__/",
    "third_party/",
    "third-party/",
    "3rdparty/",
    "external/",
    "DerivedData/",
    ".build/",
    ".gradle/",
    ".terraform/",
    "elm-stuff/",
    ".stack-work/",
];

/// Codegen file-name suffixes across ecosystems (protobuf et al). Matched
/// against the lowercased full file name.
pub(crate) const GENERATED_FILE_SUFFIXES: [&str; 3] = ["_pb.rb", "_pb2.py", ".pb.go"];

/// Multi-component generated trees, matched as an exact directory sequence
/// below the walked root. Rails migrations are history, not review surface.
pub(crate) const GENERATED_DIR_PAIRS: [(&str, &str); 1] = [("db", "migrate")];

/// Generated single files a default scan should not review: minified or
/// bundled JS (`app.min.js`, `app.bundle.js`) and codegen output matched by
/// full-name suffix (`user_pb.rb`, `user_pb2.py`, `user.pb.go`). Minified
/// rules match on the stem so every extension variant is covered. A plain
/// `bundle.js` is NOT matched -- hand-written sources win ties against
/// guesses about bundler output.
pub(crate) fn is_generated_name(name: &std::ffi::OsStr) -> bool {
    let n = name.to_string_lossy().to_ascii_lowercase();
    let Some((stem, _)) = n.rsplit_once('.') else {
        return false;
    };
    stem.ends_with(".min")
        || stem.ends_with(".bundle")
        || GENERATED_FILE_SUFFIXES.iter().any(|s| n.ends_with(s))
}

fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    if NON_PROD_DIRS.iter().any(|d| p.contains(d)) {
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
    let technical = p.ends_with("schema.rb") || p.ends_with(".lock") || p.contains("schema.rb");
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

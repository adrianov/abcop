//! ModuleSize rule: production modules must stay under MAX_LINES.
//! Classification mirrors refactor_gpt quality standards: spec/test trees,
//! test-file suffixes, lockfiles, docs and schema dumps are exempt.

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

/// Framework route tables (Rails `config/routes.rb`, engine
/// `config/routes/*.rb` and friends): declarative wiring, not review
/// surface. Matched on the repository-relative path so both the walker's
/// default prune and MR/changed-scope selection drop them.
pub(crate) fn is_route_table(rel: &std::path::Path) -> bool {
    let mut comps = rel.components();
    let file = match comps.next_back() {
        Some(c) => c,
        None => return false,
    };
    let under_config = comps.any(|c| c.as_os_str() == "config");
    if !under_config {
        return false;
    }
    let name = file.as_os_str().to_string_lossy().to_ascii_lowercase();
    let is_routes_rb = name == "routes.rb";
    let in_routes_dir = rel
        .parent()
        .and_then(|p| p.file_name())
        .map(|d| d == "routes")
        .unwrap_or(false);
    is_routes_rb || (in_routes_dir && name.ends_with(".rb"))
}

/// Third-party material: vendored dependency/build/cache trees
/// (`vendor/`, `node_modules/`, `target/`, ...), generated directory
/// sequences (`db/migrate`) and generated file names (`app.min.js`,
/// `user_pb.rb`). Matched on any path form (repo-relative or absolute)
/// so both the walker's default prune and MR/changed-scope selection
/// drop them: touching a vendored file does not make it owned production
/// code. Test trees are deliberately NOT matched -- specs stay
/// size-accountable in scoped runs.
pub(crate) fn is_third_party(path: &std::path::Path) -> bool {
    let mut prev: Option<&std::ffi::OsStr> = None;
    for comp in path.components() {
        let name = comp.as_os_str();
        let lower = name.to_string_lossy().to_ascii_lowercase();
        if GENERATED_DIRS
            .iter()
            .any(|d| lower == d.trim_end_matches('/'))
        {
            return true;
        }
        if let Some(p) = prev {
            let parent = p.to_string_lossy().to_ascii_lowercase();
            if GENERATED_DIR_PAIRS
                .iter()
                .any(|(a, b)| parent == *a && lower == *b)
            {
                return true;
            }
        }
        prev = Some(name);
    }
    path.file_name().is_some_and(is_generated_name)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rails_route_tables_are_route_files() {
        for p in [
            "config/routes.rb",
            "config/routes/api.rb",
            "engines/billing/config/routes.rb",
            "engines/billing/config/routes/admin.rb",
        ] {
            assert!(is_route_table(std::path::Path::new(p)), "{p}");
        }
    }

    #[test]
    fn ordinary_sources_are_not_route_files() {
        for p in [
            "app/models/route.rb",
            "config/application.rb",
            "config/routes_helper_spec.rb.rb",
            "routes.md",
            "app/routes_loader.rb",
            "main.rb",
        ] {
            assert!(!is_route_table(std::path::Path::new(p)), "{p}");
        }
    }

    #[test]
    fn third_party_trees_are_dropped_from_scope() {
        for p in [
            "vendor/tree-sitter-swift/src/scanner.c",
            "app/assets/node_modules/left-pad/index.js",
            "target/debug/foo.rs",
            "db/migrate/20260101120000_add_users.rb",
            "app/assets/builds/app.min.js",
            "lib/user_pb.rb",
            "/repo/vendor/x.go",
        ] {
            assert!(is_third_party(std::path::Path::new(p)), "{p}");
        }
    }

    #[test]
    fn owned_sources_stay_in_scope() {
        for p in [
            "src/main.rs",
            "spec/models/user_spec.rb",
            "test/lib/format_test.rb",
            "app/models/user.rb",
            "vendor_all/owned.rb",
            "lib/pb.rb",
        ] {
            assert!(!is_third_party(std::path::Path::new(p)), "{p}");
        }
    }
}

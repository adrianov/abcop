//! Generated / framework-owned material a default scan drops wholesale:
//! generated-tree constants plus the name/path classifiers that match them.

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

fn lowercased(os: &std::ffi::OsStr) -> String {
    os.to_string_lossy().to_ascii_lowercase()
}

/// True when some ancestor directory of `rel` (below the walked root) is a
/// `config` directory.
fn under_config(rel: &std::path::Path) -> bool {
    rel.parent().is_some_and(|dir| {
        dir.components()
            .any(|c| c.as_os_str() == std::ffi::OsStr::new("config"))
    })
}

/// True when the file sits directly inside a `routes` directory.
fn parent_is_routes_dir(rel: &std::path::Path) -> bool {
    rel.parent()
        .and_then(std::path::Path::file_name)
        .is_some_and(|dir| dir == std::ffi::OsStr::new("routes"))
}

fn route_rule(name: &str, rel: &std::path::Path) -> bool {
    name == "routes.rb" || (parent_is_routes_dir(rel) && name.ends_with(".rb"))
}

/// Framework route tables (Rails `config/routes.rb`, engine
/// `config/routes/*.rb` and friends): declarative wiring, not review
/// surface. Matched on the repository-relative path so both the walker's
/// default prune and MR/changed-scope selection drop them.
pub(crate) fn is_route_table(rel: &std::path::Path) -> bool {
    under_config(rel) && route_rule(&lowercased(rel.file_name().unwrap_or_default()), rel)
}

fn lower_of(prev: Option<&std::ffi::OsStr>) -> Option<String> {
    prev.map(lowercased)
}

fn matches_dir_pair(parent_lower: Option<String>, child: &std::ffi::OsStr) -> bool {
    let Some(parent) = parent_lower else {
        return false;
    };
    GENERATED_DIR_PAIRS
        .iter()
        .any(|(a, b)| parent == *a && lowercased(child) == *b)
}

fn generated_component_name(name: &std::ffi::OsStr) -> bool {
    let lower = lowercased(name);
    GENERATED_DIRS
        .iter()
        .any(|d| lower == d.trim_end_matches('/'))
}

fn third_party_tree(path: &std::path::Path) -> bool {
    let mut prev: Option<&std::ffi::OsStr> = None;
    for comp in path.components() {
        let name = comp.as_os_str();
        if generated_component_name(name) || matches_dir_pair(lower_of(prev), name) {
            return true;
        }
        prev = Some(name);
    }
    false
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
    third_party_tree(path) || path.file_name().is_some_and(is_generated_name)
}

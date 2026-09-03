//! Walker-side default skip policy.
//!
//! A default scan reviews production code, not third-party or machine-
//! generated material. Directory walks therefore prune:
//!
//! * test and fixture trees -- same taxonomy as ModuleAbcSize (`spec/`,
//!   `tests/`, `fixtures/`, `testdata/`, ...),
//! * vendored dependency, package-cache and build-output trees (`vendor/`,
//!   `node_modules/`, `target/`, `dist/`, `coverage/`, `venv/`,
//!   `.zig-cache/`, `zig-out/`, ...),
//! * generated directory sequences (Rails' `db/migrate`),
//! * generated single files: minified/bundled JS stems (`app.min.js`,
//!   `app.bundle.js`).
//!
//! Naming such a path explicitly on the command line opts it back in:
//! `abcop spec/`, `abcop vendor`, `abcop db/migrate` walk exactly what was
//! asked for. Any other root (`.`, `src`, an absolute path) keeps the
//! defaults active below it.

use crate::modulesize::{GENERATED_DIR_PAIRS, GENERATED_DIRS, NON_PROD_DIRS, is_generated_name};

/// Drop a leading `./` so root matching works against the walker's entry
/// spellings (`Path` normalizes `./` away internally, raw prefixes don't).
/// The bare `.` becomes the empty path, which prefix-matches everything --
/// exactly what a cwd root should do.
fn normalize_root(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if s == "." {
        return std::path::PathBuf::new();
    }
    match s.strip_prefix("./") {
        Some(rest) => std::path::PathBuf::from(rest),
        None => p.to_path_buf(),
    }
}

/// Walker-side default policy: an entry below a walked root is skipped when
/// one of its directory components is a test/fixture dir, a vendored/build/
/// generated dir, part of a generated sequence (`db/migrate`) -- or, for
/// files, when its stem marks it as minified/bundled output. Component
/// equality avoids substring traps like `contest/` matching `test/`.
///
/// The matched root itself is exempt from the check when its own final
/// segment names a skipped tree: the user targeted that material directly.
pub(crate) fn skipped_by_default(path: &std::path::Path, roots: &[std::path::PathBuf]) -> bool {
    let path = normalize_root(path);
    let Some(root) = roots.iter().map(|r| normalize_root(r)).find_best(&path) else {
        return false;
    };
    if any_component_matches(&root) {
        return false;
    }
    let Ok(rel) = path.strip_prefix(root.as_path()) else {
        return false;
    };

    if is_generated_name(rel.file_name().unwrap_or_default())
        || crate::modulesize::is_route_table(rel)
    {
        return true;
    }
    rel.parent().is_some_and(any_component_matches)
}

/// Longest normalized root that is a prefix of `path`, if any.
trait RootPick {
    fn find_best(self, path: &std::path::Path) -> Option<std::path::PathBuf>;
}

impl<I: Iterator<Item = std::path::PathBuf>> RootPick for I {
    fn find_best(self, path: &std::path::Path) -> Option<std::path::PathBuf> {
        self.filter(|r| path.starts_with(r.as_path()))
            .max_by_key(|r| r.as_os_str().len())
    }
}

/// Sliding-window check over a directory sequence: any single component in
/// the skip taxonomy, or any adjacent pair forming a generated sequence.
fn any_component_matches(path: &std::path::Path) -> bool {
    let mut prev: Option<&std::ffi::OsStr> = None;
    for comp in path.components() {
        let name = comp.as_os_str();
        if is_skip_dir(name) || prev.is_some_and(|p| is_skip_pair(p, name)) {
            return true;
        }
        prev = Some(name);
    }
    false
}

fn is_skip_dir(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    NON_PROD_DIRS
        .iter()
        .chain(GENERATED_DIRS.iter())
        .any(|d| name == d.trim_end_matches('/'))
}

fn is_skip_pair(parent: &std::ffi::OsStr, child: &std::ffi::OsStr) -> bool {
    let parent = parent.to_string_lossy().to_ascii_lowercase();
    let child = child.to_string_lossy().to_ascii_lowercase();
    GENERATED_DIR_PAIRS
        .iter()
        .any(|(p, c)| parent == *p && child == *c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn roots<const N: usize>(paths: [&str; N]) -> Vec<std::path::PathBuf> {
        paths.into_iter().map(std::path::PathBuf::from).collect()
    }

    #[test]
    fn skips_test_fixture_and_vendored_dirs_below_root() {
        let r = roots(["."]);
        assert!(skipped_by_default(Path::new("./spec/models/x_spec.rb"), &r));
        assert!(skipped_by_default(Path::new("./fixtures/probe.rb"), &r));
        assert!(skipped_by_default(
            Path::new("./vendor/bundle/gem/lib.rb"),
            &r
        ));
        assert!(skipped_by_default(Path::new("./node_modules/pkg/i.js"), &r));
    }

    #[test]
    fn skips_db_migrate_sequence_only() {
        let r = roots(["."]);
        assert!(skipped_by_default(
            Path::new("./db/migrate/001_create.rb"),
            &r
        ));
        assert!(!skipped_by_default(Path::new("./db/seeds.rb"), &r));
        assert!(!skipped_by_default(
            Path::new("./app/db/migrate_like.rb"),
            &r
        ));
    }

    #[test]
    fn skip_named_roots_are_trusted_wholesale() {
        assert!(!skipped_by_default(
            Path::new("vendor/gem/lib.rb"),
            &roots(["vendor"])
        ));
        assert!(!skipped_by_default(
            Path::new("./spec/models/x_spec.rb"),
            &roots(["spec"])
        ));
        assert!(!skipped_by_default(
            Path::new("./db/migrate/001_create.rb"),
            &roots([".", "db/migrate"])
        ));
    }

    #[test]
    fn other_roots_keep_defaults_active() {
        // app/ is not a skip-named tree: defaults still apply inside it,
        // even though it was named explicitly next to `.`.
        let r = roots([".", "./app"]);
        assert!(skipped_by_default(Path::new("./app/spec/a.rb"), &r));
        assert!(skipped_by_default(Path::new("./spec/a.rb"), &r));
    }

    #[test]
    fn component_equality_no_substring_traps() {
        let r = roots(["."]);
        assert!(!skipped_by_default(Path::new("./src/latest/a.rs"), &r));
        assert!(!skipped_by_default(Path::new("./src/contest/t.rb"), &r));
    }

    #[test]
    fn minified_files_are_generated() {
        let r = roots(["."]);
        assert!(skipped_by_default(Path::new("./public/app.min.js"), &r));
        assert!(skipped_by_default(Path::new("./public/app.bundle.js"), &r));
        assert!(!skipped_by_default(Path::new("./public/app.js"), &r));
        assert!(is_generated_name(std::ffi::OsStr::new("x.min.mjs")));
        assert!(!is_generated_name(std::ffi::OsStr::new("bundle.rs")));
    }
    #[test]
    fn expanded_vendor_dirs_and_codegen_suffixes() {
        let r = roots(["."]);
        assert!(skipped_by_default(
            Path::new("./third_party/lib/foo.rb"),
            &r
        ));
        assert!(skipped_by_default(
            Path::new("./third-party/lib/foo.rb"),
            &r
        ));
        assert!(skipped_by_default(Path::new("./.terraform/main.tf.rb"), &r));
        assert!(skipped_by_default(Path::new("./app/models/user_pb.rb"), &r));
        assert!(!skipped_by_default(Path::new("./app/models/user.rb"), &r));
        assert!(skipped_by_default(
            Path::new("./lib/models/user.freezed.dart"),
            &r
        ));
        assert!(skipped_by_default(
            Path::new("./lib/models/user.g.dart"),
            &r
        ));
        assert!(!skipped_by_default(Path::new("./lib/models/user.dart"), &r));
    }
}

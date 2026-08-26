//! Per-file analysis pipeline shared by all language backends.

use crate::cache;
use crate::directives;
use crate::git_changes;
pub(crate) use crate::paths::Lang;
use crate::paths::{lang_for, parse_file_lang};
use crate::srcbuf::{SrcBuf, load_src};

#[derive(Debug)]
pub(crate) struct Checks {
    pub want_abc: bool,
    pub want_used: bool,
    pub want_never: bool,
}

impl Checks {
    pub fn new(only: Option<&str>) -> Self {
        Self {
            want_abc: only.is_none_or(|o| o == "abc"),
            want_used: only.is_none_or(|o| o == "used-once"),
            want_never: only.is_none_or(|o| o == "never-used"),
        }
    }
}

fn blank_with(path: &std::path::Path) -> crate::output::FileResult {
    crate::output::FileResult {
        path: path.display().to_string(),
        abc: Vec::new(),
        used_once: Vec::new(),
        never_used: Vec::new(),
        oversize: None,
    }
}

fn reparsed<'a>(src_bytes: &'a [u8], lang: Lang) -> Option<crate::model::FileModel<'a>> {
    let tree = parse_file_lang(src_bytes, lang)?;
    Some(crate::model::build(src_bytes, tree))
}

pub(crate) fn analyze_one(
    path: &std::path::Path,
    only: Option<&str>,
    max: f64,
    changeset: Option<&git_changes::Changeset>,
    cache: Option<&cache::Cache>,
) -> crate::output::FileResult {
    let Some((mut r, src_buf)) = loaded_result(path) else {
        return blank_with(path);
    };
    // Cache stores raw per-file results; scope narrowing is per-run and
    // must also apply when the analysis itself comes from the cache.
    let hit = cache.and_then(|c| cache_hit(c, path, &src_buf, only, max));
    if let Some(mut hit) = hit {
        apply_changeset(changeset, &mut hit, &src_buf);
        return hit;
    }

    let file_lang = lang_for(path);
    let Some(tree) = parse_file_lang(&src_buf, file_lang) else {
        return r;
    };
    let checks = Checks::new(only);
    if file_lang.is_clike() {
        clike_arm(&mut r, file_lang, &src_buf, &tree, &checks, max);
    } else if !non_clike_arm(&mut r, file_lang, &src_buf, tree, &checks, max) {
        // Unparsable Ruby tree: report the blank result without caching it.
        return r;
    }
    store_result(cache, path, &src_buf, only, max, &r);
    apply_changeset(changeset, &mut r, &src_buf);
    r
}

/// Blank result plus source bytes; None when the file cannot be read.
fn loaded_result(path: &std::path::Path) -> Option<(crate::output::FileResult, SrcBuf)> {
    let mut r = blank_with(path);
    let src_buf = load_src(path).ok()?;
    r.oversize = std::str::from_utf8(&src_buf)
        .ok()
        .and_then(|t| crate::modulesize::offense(t, &r.path));
    Some((r, src_buf))
}

/// Cached FileResult rebuilt from the store; None on a miss.
fn cache_hit(
    cache: &cache::Cache,
    path: &std::path::Path,
    src: &[u8],
    only: Option<&str>,
    max: f64,
) -> Option<crate::output::FileResult> {
    let key = cache.file_key(path, src, only, max);
    let (abc, used_once, never_used, oversize) = cache.get(&key)?;
    Some(crate::output::FileResult {
        path: path.display().to_string(),
        abc,
        used_once,
        never_used,
        oversize,
    })
}

fn store_result(
    cache: Option<&cache::Cache>,
    path: &std::path::Path,
    src: &[u8],
    only: Option<&str>,
    max: f64,
    r: &crate::output::FileResult,
) {
    if let Some(cache) = cache {
        let key = cache.file_key(path, src, only, max);
        cache.store(&key, &r.abc, &r.used_once, &r.never_used, r.oversize);
    }
}

/// C-family backend: ABC only, honoring inline suppression directives.
fn clike_arm(
    r: &mut crate::output::FileResult,
    lang: Lang,
    src: &[u8],
    tree: &tree_sitter::Tree,
    checks: &Checks,
    max: f64,
) {
    let dirs = directives::parse(&String::from_utf8_lossy(src));
    if checks.want_abc {
        r.abc = suppressed(crate::clike::analyze(src, tree, lang, max), |o| {
            dirs.suppresses_abc(o.line)
        });
    }
}

/// Ruby/Rust backends. False when the Ruby reparse fails (no usable model).
fn non_clike_arm(
    r: &mut crate::output::FileResult,
    lang: Lang,
    src: &[u8],
    tree: tree_sitter::Tree,
    checks: &Checks,
    max: f64,
) -> bool {
    match lang {
        Lang::Rust => {
            let fm = crate::rustlang::build(src, tree);
            if checks.want_abc {
                r.abc = crate::rustlang::analyze(&fm, max);
            }
            if checks.want_used {
                r.used_once = crate::rustlang::used_once_offenses(&fm);
            }
            if checks.want_never {
                r.never_used = crate::rustlang::never_used_offenses(&fm);
            }
            true
        }
        Lang::Py => {
            let dirs = directives::parse(&String::from_utf8_lossy(src));
            let fm = crate::pylang::build(src, tree);
            if checks.want_abc {
                r.abc = suppressed(crate::pylang::analyze(&fm, max), |o| {
                    dirs.suppresses_abc(o.line)
                });
            }
            if checks.want_used {
                r.used_once =
                    suppressed(crate::pylang::used_once_offenses(&fm), |o| {
                        dirs.suppresses_all(o.line)
                    });
            }
            if checks.want_never {
                r.never_used = crate::pylang::never_used_offenses(&fm);
            }
            true
        }
        Lang::Go => {
            let dirs = directives::parse(&String::from_utf8_lossy(src));
            let fm = crate::golang::build(src, tree);
            if checks.want_abc {
                r.abc = suppressed(crate::golang::analyze(&fm, max), |o| {
                    dirs.suppresses_abc(o.line)
                });
            }
            if checks.want_used {
                r.used_once =
                    suppressed(crate::golang::used_once_offenses(&fm), |o| {
                        dirs.suppresses_all(o.line)
                    });
            }
            if checks.want_never {
                r.never_used = crate::golang::never_used_offenses(&fm);
            }
            true
        }
        Lang::Php => {
            let dirs = directives::parse(&String::from_utf8_lossy(src));
            let fm = crate::phplang::build(src, tree);
            if checks.want_abc {
                r.abc = suppressed(crate::phplang::analyze(&fm, max), |o| {
                    dirs.suppresses_abc(o.line)
                });
            }
            if checks.want_used {
                r.used_once =
                    suppressed(crate::phplang::used_once_offenses(&fm), |o| {
                        dirs.suppresses_all(o.line)
                    });
            }
            if checks.want_never {
                r.never_used = crate::phplang::never_used_offenses(&fm);
            }
            true
        }
        Lang::Ruby => ruby_arm(r, src, checks, max),
        _ => unreachable!("non-clike languages are Ruby, Rust, Python, Go and PHP"),
    }
}

/// Ruby backend: directives-filtered ABC and used-once, plain never-used.
/// False when the Ruby reparse fails (no usable model).
fn ruby_arm(r: &mut crate::output::FileResult, src: &[u8], checks: &Checks, max: f64) -> bool {
    let dirs = directives::parse(&String::from_utf8_lossy(src));
    let Some(fm) = reparsed(src, Lang::Ruby) else {
        return false;
    };
    if checks.want_abc {
        r.abc = suppressed(crate::abc::analyze(&fm, max), |o| {
            dirs.suppresses_abc(o.line)
        });
    }
    if checks.want_used {
        r.used_once = suppressed(crate::used_once::analyze(&fm), |o| {
            dirs.suppresses_all(o.line)
        });
    }
    if checks.want_never {
        r.never_used = crate::never_used::analyze(&fm);
    }
    true
}

/// Drops entries the inline directives suppress.
fn suppressed<T>(v: Vec<T>, keep: impl Fn(&T) -> bool) -> Vec<T> {
    v.into_iter().filter(|o| !keep(o)).collect()
}

/// Narrows a fresh result to the lines touched in this working-tree change.
fn apply_changeset(
    changeset: Option<&git_changes::Changeset>,
    r: &mut crate::output::FileResult,
    src: &[u8],
) {
    let Some(cs) = changeset else { return };
    let Some(rel) = cs.rel_of(&r.path) else {
        return;
    };
    r.abc.retain(|o| cs.span_selected(rel, o.line, o.end_line));
    r.used_once.retain(|o| cs.line_selected(rel, o.line));
    r.never_used.retain(|o| cs.line_selected(rel, o.line));
    // Scoped runs gate ModuleSize on refactor-scale diffs only -- for any
    // module, spec or production: a >=100-line diff invites the size
    // conversation even in tests, while small patches into legacy giants
    // do not.
    let count = changed_line_count(cs, rel);
    let refactor_scale = count >= crate::modulesize::MIN_REVIEW_REFACTOR_LINES;
    if r.oversize.is_some() {
        if !refactor_scale {
            r.oversize = None;
        }
    } else if refactor_scale && crate::modulesize::is_test_path(rel) {
        let text = std::str::from_utf8(src).unwrap_or("");
        let lines = crate::modulesize::effective_lines(text, rel);
        r.oversize = (lines >= crate::modulesize::MAX_LINES).then_some(lines);
    }
}

/// Touched-line count for a repo-relative path; untracked files count as
/// fully changed.
fn changed_line_count(cs: &git_changes::Changeset, rel: &str) -> usize {
    match cs.files.get(rel) {
        Some(git_changes::Lines::All) => usize::MAX,
        Some(git_changes::Lines::Ranges(set)) => set.len(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// Regression: a warm-cache hit used to skip apply_changeset, so
    /// scoped runs served un-narrowed results (e.g. ModuleSize from a
    /// three-line diff). Both paths must narrow identically.
    #[test]
    fn cache_hits_are_scope_narrowed_like_fresh_analysis() {
        let dir = std::env::temp_dir().join(format!(
            "abcop_pipeline_narrow_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let file = dir.join("app/models_big.rb");
        let mut src = String::new();
        for i in 1..=60 {
            src.push_str(&format!("def filler_{i}\n  x_{i} = 1\n  unused_{i} = 2\nend\n\n"));
        }
        std::fs::write(&file, &src).unwrap();

        let cs = git_changes::Changeset {
            root: dir.display().to_string(),
            files: BTreeMap::from([(
                "app/models_big.rb".to_string(),
                git_changes::Lines::Ranges(BTreeSet::from([1, 2])),
            )]),
        };

        // Fresh analysis path.
        let first = analyze_one(&file, None, 17.0, Some(&cs), None);
        assert_eq!(first.oversize, None, "3-line diff must not flag size");

        // Warm cache path over a run-wide cache directory.
        let cache = crate::cache::Cache::open_at(&dir.join(".cache")).expect("cache");
        let _ = analyze_one(&file, None, 17.0, Some(&cs), Some(&cache));
        let second = analyze_one(&file, None, 17.0, Some(&cs), Some(&cache));
        assert_eq!(second.oversize, None, "cache hit must stay narrowed");
        // Warm-cache diagnostics must equal fresh-analysis diagnostics.
        assert_eq!(first.abc, second.abc);
        assert_eq!(first.used_once, second.used_once);
        assert_eq!(first.never_used, second.never_used);

        let _ = std::fs::remove_dir_all(&dir);
    }


    fn cs_with(rel: &str, lines: git_changes::Lines) -> git_changes::Changeset {
        git_changes::Changeset {
            root: "/repo".into(),
            files: BTreeMap::from([(rel.into(), lines)]),
        }
    }

    fn result_with_oversize() -> crate::output::FileResult {
        crate::output::FileResult {
            path: "/repo/app/models/big.rb".into(),
            abc: Vec::new(),
            used_once: Vec::new(),
            never_used: Vec::new(),
            oversize: Some(228),
        }
    }

    #[test]
    fn small_diffs_into_large_modules_are_not_flagged() {
        let mut set = BTreeSet::from([3, 4, 5]);
        set.insert(9);
        let cs = cs_with("app/models/big.rb", git_changes::Lines::Ranges(set));
        let mut r = result_with_oversize();
        apply_changeset(Some(&cs), &mut r, b"");
        assert_eq!(r.oversize, None);
    }

    #[test]
    fn refactor_scale_diffs_keep_module_size() {
        let set: BTreeSet<usize> = (1..=120).collect();
        let cs = cs_with("app/models/big.rb", git_changes::Lines::Ranges(set));
        let mut r = result_with_oversize();
        apply_changeset(Some(&cs), &mut r, b"");
        assert_eq!(r.oversize, Some(228));
    }

    #[test]
    fn new_files_count_as_fully_changed() {
        let cs = cs_with("app/models/big.rb", git_changes::Lines::All);
        let mut r = result_with_oversize();
        apply_changeset(Some(&cs), &mut r, b"");
        assert_eq!(r.oversize, Some(228));
    }

    #[test]
    fn big_scoped_diff_makes_specs_size_accountable() {
        let dir = std::env::temp_dir().join(format!(
            "abcop_pipeline_specsize_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("test")).unwrap();
        let file = dir.join("test/commit_plan_finalize_test.rb");
        let mut src = String::new();
        for i in 1..=60 {
            src.push_str(&format!("def test_fill_{i}\n  assert true\nend\n\n"));
        }
        std::fs::write(&file, &src).unwrap();
        let root = dir.display().to_string();

        let mk = |n: usize| git_changes::Changeset {
            root: root.clone(),
            files: BTreeMap::from([(
                "test/commit_plan_finalize_test.rb".to_string(),
                git_changes::Lines::Ranges((1..=n).collect()),
            )]),
        };

        // small diff: test-tree exemption holds
        let mut r = result_with_oversize_for(&file);
        apply_changeset(Some(&mk(3)), &mut r, &src.as_bytes());
        assert_eq!(r.oversize, None);

        // refactor-scale diff: spec becomes size-accountable
        let mut r = result_with_oversize_for(&file);
        apply_changeset(Some(&mk(120)), &mut r, &src.as_bytes());
        assert_eq!(r.oversize, Some(src.lines().count()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn result_with_oversize_for(path: &std::path::Path) -> crate::output::FileResult {
        let (r, _) = loaded_result(path).expect("fixture loads");
        r
    }

    #[test]
    fn full_scans_keep_module_size() {
        let mut r = result_with_oversize();
        apply_changeset(None, &mut r, b"");
        assert_eq!(r.oversize, Some(228));
    }
}

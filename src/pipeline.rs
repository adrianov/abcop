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
    let hit = cache.and_then(|c| cache_hit(c, path, &src_buf, only, max));
    if let Some(hit) = hit {
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
    apply_changeset(changeset, &mut r);
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
        Lang::Ruby => ruby_arm(r, src, checks, max),
        _ => unreachable!("non-clike languages are Ruby and Rust only"),
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
fn apply_changeset(changeset: Option<&git_changes::Changeset>, r: &mut crate::output::FileResult) {
    let Some(cs) = changeset else { return };
    let Some(rel) = cs.rel_of(&r.path) else {
        return;
    };
    r.abc.retain(|o| cs.span_selected(rel, o.line, o.end_line));
    r.used_once.retain(|o| cs.line_selected(rel, o.line));
    r.never_used.retain(|o| cs.line_selected(rel, o.line));
}

//! Per-file analysis pipeline: scope resolution feeding the backend
//! dispatch (`backends`), cache round-trip, and changeset narrowing
//! (`narrow`).

use crate::cache;
use crate::git_changes;
use crate::paths::{lang_for, parse_file_lang, Lang};
use crate::srcbuf::{load_src, SrcBuf};
mod backends;
mod non_clike;
mod narrow;

#[cfg(test)]
mod tests;

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
        narrow::apply(changeset, &mut hit, &src_buf);
        return hit;
    }

    let file_lang = lang_for(path);
    let Some(tree) = parse_file_lang(&src_buf, file_lang) else {
        return r;
    };
    let checks = Checks::new(only);
    if file_lang.is_clike() {
        backends::clike_arm(&mut r, file_lang, &src_buf, &tree, &checks, max);
    } else if !backends::non_clike_arm(&mut r, file_lang, &src_buf, tree, &checks, max) {
        // Unparsable Ruby tree: report the blank result without caching it.
        return r;
    }
    store_result(cache, path, &src_buf, only, max, &r);
    narrow::apply(changeset, &mut r, &src_buf);
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

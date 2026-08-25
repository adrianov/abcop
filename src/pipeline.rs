//! Per-file analysis pipeline shared by all language backends.

use std::fs;

use crate::directives;
use crate::cache;
use crate::git_changes;
pub(crate) use crate::paths::Lang;
use crate::paths::{lang_for, parse_file_lang};

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
    let mut r = blank_with(path);
    let Ok(src_buf) = load_src(path) else { return r };
    let src_bytes: &[u8] = &src_buf;
    r.oversize = std::str::from_utf8(src_bytes)
        .ok()
        .and_then(|t| crate::modulesize::offense(t, &r.path));

    let key = cache
        .map(|c| c.file_key(path, src_bytes, only, max))
        .unwrap_or_default();
    if let (Some(cache), false) = (cache, key.is_empty())
        && let Some((abc, used_once, never_used, oversize)) = cache.get(&key)
    {
        return crate::output::FileResult {
            path: path.display().to_string(),
            abc,
            used_once,
            never_used,
            oversize,
        };
    }

    let Some(tree) = parse_file_lang(src_bytes, lang_for(path)) else {
        return r;
    };
    let checks = Checks::new(only);

    let file_lang = lang_for(path);
    match file_lang {
        Lang::Rust => {
            let fm = crate::rustlang::build(src_bytes, tree);
            if checks.want_abc {
                r.abc = crate::rustlang::analyze(&fm, max);
            }
            if checks.want_used {
                r.used_once = crate::rustlang::used_once_offenses(&fm);
            }
            if checks.want_never {
                r.never_used = crate::rustlang::never_used_offenses(&fm);
            }
            if let Some(cache) = cache {
                cache.store(
                    &key,
                    &r.abc, &r.used_once, &r.never_used, r.oversize,
                );
            }
        }
        Lang::Ruby => {
            let dirs = directives::parse(&String::from_utf8_lossy(src_bytes));
            let Some(fm) = reparsed(src_bytes, file_lang) else { return r };
            if checks.want_abc {
                r.abc = crate::abc::analyze(&fm, max)
                    .into_iter()
                    .filter(|o| !dirs.suppresses_abc(o.line))
                    .collect();
            }
            if checks.want_used {
                r.used_once = crate::used_once::analyze(&fm)
                    .into_iter()
                    .filter(|o| !dirs.suppresses_all(o.line))
                    .collect();
            }
            if checks.want_never {
                r.never_used = crate::never_used::analyze(&fm);
            }
            if let Some(cache) = cache {
                cache.store(
                    &key,
                    &r.abc, &r.used_once, &r.never_used, r.oversize,
                );
            }
        }
    }

    if let Some(cs) = changeset
        && let Some(rel) = cs.rel_of(&r.path)
    {
        r.abc.retain(|o| cs.span_selected(rel, o.line, o.end_line));
        r.used_once.retain(|o| cs.line_selected(rel, o.line));
        r.never_used.retain(|o| cs.line_selected(rel, o.line));
    }
    r
}

/// Hybrid file input: memory-map large files (zero-copy from the page
/// cache), plain pre-sized read for small ones where mmap setup would cost
/// more than the copy.
const MMAP_THRESHOLD: u64 = 64 * 1024;

pub(crate) enum SrcBuf {
    Heap(Vec<u8>),
    Mapped(memmap2::Mmap),
}

impl std::ops::Deref for SrcBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            SrcBuf::Heap(b) => b,
            SrcBuf::Mapped(m) => m,
        }
    }
}

pub(crate) fn load_src(path: &std::path::Path) -> std::io::Result<SrcBuf> {
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len >= MMAP_THRESHOLD {
        // SAFETY: read-only mapping of a regular file opened for reading.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        return Ok(SrcBuf::Mapped(map));
    }
    let mut buf = Vec::with_capacity(len as usize);
    {
        use std::io::Read;
        (&file).read_to_end(&mut buf)?;
    }
    Ok(SrcBuf::Heap(buf))
}

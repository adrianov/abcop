//! JavaScript embedded in HTML and template hosts (ERB, Slim, Jinja, …).
//!
//! Host files are not scored as markup: script slices are extracted, template
//! holes blanked, then the usual JS/TS clike metrics run with positions
//! remapped back to the host.

mod attrs;
mod blank;
mod extract;
mod filter;
mod host;
#[cfg(test)]
mod tests;

use crate::abc::{AbcOffense, Limits};
use crate::directives::{self, Directives};
use crate::modulesize;
use crate::never_used::NeverUsedOffense;
use crate::output::FileResult;
use crate::paths::{Lang, parse_file_lang};
use crate::used_once::UsedOnceOffense;

pub(crate) use host::is_host;

struct Want {
    abc: bool,
    used: bool,
    never: bool,
}

impl Want {
    fn new(only: Option<&str>) -> Self {
        Self {
            abc: only.is_none_or(|o| o == "abc"),
            used: only.is_none_or(|o| o == "used-once"),
            never: only.is_none_or(|o| o == "never-used"),
        }
    }
}

/// Analyse JS/TS embedded in a template host into `r`.
pub(crate) fn analyze(
    r: &mut FileResult,
    path: &std::path::Path,
    src: &[u8],
    only: Option<&str>,
    limits: Limits,
) {
    let want = Want::new(only);
    finish(
        r,
        src,
        &directives::parse(&String::from_utf8_lossy(src)),
        &want,
        limits,
        score_all(path, src, &want),
    );
}

fn finish(
    r: &mut FileResult,
    src: &[u8],
    dirs: &Directives,
    want: &Want,
    limits: Limits,
    scored: (Vec<AbcOffense>, Vec<UsedOnceOffense>, Vec<NeverUsedOffense>),
) {
    let (all_scores, used, never) = scored;
    r.module_abc = modulesize::from_scores(
        &all_scores,
        &r.path,
        std::str::from_utf8(src).unwrap_or(""),
        limits.module,
    );
    if want.abc {
        r.abc = keep_abc(dirs, all_scores, limits.method);
    }
    if want.used {
        r.used_once = used
            .into_iter()
            .filter(|o| !dirs.suppresses_all(o.line))
            .collect();
    }
    if want.never {
        r.never_used = never;
    }
}

fn score_all(
    path: &std::path::Path,
    src: &[u8],
    want: &Want,
) -> (
    Vec<AbcOffense>,
    Vec<UsedOnceOffense>,
    Vec<NeverUsedOffense>,
) {
    let mut all_scores = Vec::new();
    let mut used = Vec::new();
    let mut never = Vec::new();
    for frag in extract::fragments(path, src) {
        score_one(&frag, want, &mut all_scores, &mut used, &mut never);
    }
    (all_scores, used, never)
}

fn score_one(
    frag: &extract::Fragment,
    want: &Want,
    all_scores: &mut Vec<AbcOffense>,
    used: &mut Vec<UsedOnceOffense>,
    never: &mut Vec<NeverUsedOffense>,
) {
    let Some(tree) = parse_file_lang(&frag.code, frag.lang) else {
        return;
    };
    all_scores.extend(
        crate::clike::all_scores(&frag.code, &tree, frag.lang)
            .into_iter()
            .map(|o| remap_abc(frag, o)),
    );
    collect_usage(frag, &tree, want, used, never);
}

fn collect_usage(
    frag: &extract::Fragment,
    tree: &tree_sitter::Tree,
    want: &Want,
    used: &mut Vec<UsedOnceOffense>,
    never: &mut Vec<NeverUsedOffense>,
) {
    if !(want.used || want.never) || !js_family(frag.lang) {
        return;
    }
    let scopes = crate::clike::collect_scopes(&frag.code, tree, frag.lang);
    if want.used {
        used.extend(usage_once(frag, &scopes).into_iter().map(|o| remap_used(frag, o)));
    }
    if want.never {
        never.extend(usage_never(frag, &scopes).into_iter().map(|o| remap_never(frag, o)));
    }
}

fn js_family(lang: Lang) -> bool {
    matches!(lang, Lang::Js | Lang::Ts | Lang::Tsx)
}

fn usage_once(
    frag: &extract::Fragment,
    scopes: &crate::clike::JsScopes,
) -> Vec<UsedOnceOffense> {
    if frag.report_root {
        crate::clike::used_once_offenses_embed(scopes)
    } else {
        crate::clike::used_once_offenses(scopes, frag.lang)
    }
}

fn usage_never(
    frag: &extract::Fragment,
    scopes: &crate::clike::JsScopes,
) -> Vec<NeverUsedOffense> {
    if frag.report_root {
        crate::clike::never_used_offenses_embed(scopes)
    } else {
        crate::clike::never_used_offenses(scopes, frag.lang)
    }
}

fn keep_abc(dirs: &Directives, all: Vec<AbcOffense>, max: f64) -> Vec<AbcOffense> {
    all.into_iter()
        .filter(|o| o.score > max && !dirs.suppresses_abc(o.line))
        .collect()
}

fn remap_pos(frag: &extract::Fragment, line: usize, column: usize) -> (usize, usize) {
    if line <= 1 {
        (frag.start_line, frag.start_col.saturating_add(column))
    } else {
        (frag.start_line + line - 1, column)
    }
}

fn remap_abc(frag: &extract::Fragment, mut o: AbcOffense) -> AbcOffense {
    let (line, column) = remap_pos(frag, o.line, o.column);
    o.end_line = if o.end_line <= 1 {
        frag.start_line
    } else {
        frag.start_line + o.end_line - 1
    };
    o.line = line;
    o.column = column;
    o
}

fn remap_used(frag: &extract::Fragment, mut o: UsedOnceOffense) -> UsedOnceOffense {
    let (line, column) = remap_pos(frag, o.line, o.column);
    o.line = line;
    o.column = column;
    o
}

fn remap_never(frag: &extract::Fragment, mut o: NeverUsedOffense) -> NeverUsedOffense {
    let (line, column) = remap_pos(frag, o.line, o.column);
    o.line = line;
    o.column = column;
    o
}

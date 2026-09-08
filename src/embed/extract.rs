//! Extract `<script>` bodies from HTML-like hosts.

use crate::paths::Lang;

use super::attrs::{
    find_byte, find_subslice, has_token, is_js_mime, to_ascii_lower, type_or_lang,
};
use super::blank::blank_holes;
use super::filter::filter_blocks;
use super::host::{is_module_host, is_whole_js_file};

/// One script slice ready for the JS/TS backend.
pub(super) struct Fragment {
    pub lang: Lang,
    pub code: Vec<u8>,
    /// 1-based host line of the first code byte.
    pub start_line: usize,
    /// 0-based host column of the first code byte.
    pub start_col: usize,
    /// Report root-scope locals (classic inline scripts, not modules/SFCs).
    pub report_root: bool,
}

pub(super) fn fragments(path: &std::path::Path, src: &[u8]) -> Vec<Fragment> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if is_whole_js_file(&name) {
        return whole_file(src, lang_for_name(&name));
    }
    let mut out = script_tags(src, is_module_host(&name));
    out.extend(filter_blocks(src));
    out
}

fn lang_for_name(name: &str) -> Lang {
    if name.contains(".tsx.") || name.contains(".jsx.") {
        Lang::Tsx
    } else if name.contains(".ts.") || name.contains(".mts.") || name.contains(".cts.") {
        Lang::Ts
    } else {
        Lang::Js
    }
}

fn whole_file(src: &[u8], lang: Lang) -> Vec<Fragment> {
    let mut code = src.to_vec();
    blank_holes(&mut code);
    vec![Fragment {
        lang,
        code,
        start_line: 1,
        start_col: 0,
        report_root: false,
    }]
}

fn script_tags(src: &[u8], module_host: bool) -> Vec<Fragment> {
    let mut out = Vec::new();
    let lower = to_ascii_lower(src);
    let mut i = 0;
    while let Some(open_at) = next_script_open(&lower, i) {
        i = match push_script(&mut out, src, &lower, open_at, module_host) {
            Some(next) => next,
            None => break,
        };
    }
    out
}

/// Returns the next search index, or None when the open tag is truncated.
fn push_script(
    out: &mut Vec<Fragment>,
    src: &[u8],
    lower: &[u8],
    open_at: usize,
    module_host: bool,
) -> Option<usize> {
    let (gt_at, attrs) = script_open_attrs(lower, open_at)?;
    if !is_js_script(attrs) {
        return Some(gt_at + 1);
    }
    let body_at = gt_at + 1;
    let close = find_subslice(&lower[body_at..], b"</script")?;
    let body_end = body_at + close;
    if let Some(frag) = body_frag(src, body_at, body_end, attrs, module_host) {
        out.push(frag);
    }
    Some(body_end + b"</script".len())
}

fn body_frag(
    src: &[u8],
    start: usize,
    end: usize,
    attrs: &[u8],
    module_host: bool,
) -> Option<Fragment> {
    push_ready(
        src,
        start,
        end,
        script_lang(attrs),
        !module_host && !is_module_script(attrs),
    )
}

fn next_script_open(lower: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < lower.len() {
        let rel = find_subslice(&lower[i..], b"<script")?;
        let open_at = i + rel;
        let after = open_at + b"<script".len();
        if after < lower.len() && is_name_continue(lower[after]) {
            i = after;
            continue;
        }
        return Some(open_at);
    }
    None
}

fn is_name_continue(n: u8) -> bool {
    n.is_ascii_alphanumeric() || n == b'-' || n == b'_'
}

fn script_open_attrs(lower: &[u8], open_at: usize) -> Option<(usize, &[u8])> {
    let after_name = open_at + b"<script".len();
    let gt = find_byte(&lower[after_name..], b'>')?;
    let gt_at = after_name + gt;
    Some((gt_at, &lower[after_name..gt_at]))
}

fn is_js_script(attrs: &[u8]) -> bool {
    type_or_lang(attrs).is_none_or(|v| is_js_mime(&v))
}

fn script_lang(attrs: &[u8]) -> Lang {
    match type_or_lang(attrs).as_deref() {
        Some(v) if v.contains("tsx") || v == "tsx" => Lang::Tsx,
        Some(v) if v.contains("typescript") || v == "ts" => Lang::Ts,
        _ => Lang::Js,
    }
}

fn is_module_script(attrs: &[u8]) -> bool {
    has_token(attrs, b"setup") || matches!(type_or_lang(attrs).as_deref(), Some("module"))
}

fn push_ready(
    src: &[u8],
    start: usize,
    end: usize,
    lang: Lang,
    report_root: bool,
) -> Option<Fragment> {
    if start >= end {
        return None;
    }
    let mut code = src[start..end].to_vec();
    if code.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    blank_holes(&mut code);
    let (start_line, start_col) = byte_line_col(src, start);
    Some(Fragment {
        lang,
        code,
        start_line,
        start_col,
        report_root,
    })
}

fn byte_line_col(src: &[u8], byte: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 0usize;
    for &b in &src[..byte.min(src.len())] {
        if b == b'\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

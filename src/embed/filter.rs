//! Slim / Haml / Pug-style `javascript:` / `:javascript` indented blocks.

use crate::paths::Lang;

use super::blank::blank_holes;
use super::extract::Fragment;

pub(super) fn filter_blocks(src: &[u8]) -> Vec<Fragment> {
    let text = String::from_utf8_lossy(src).into_owned();
    let mut out = Vec::new();
    let mut i = 0;
    let starts = line_offsets(&text);
    while i < starts.len() {
        let line = line_at(&text, &starts, i);
        if !is_filter_or_script(line) {
            i += 1;
            continue;
        }
        let base = indent_width(line);
        let filter_line = i + 1;
        let inline = inline_filter_code(line);
        i += 1;
        if let Some(frag) = take_filter_body(&text, &starts, &mut i, base, filter_line, inline) {
            out.push(frag);
        }
    }
    out
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' && i + 1 < text.len() {
            starts.push(i + 1);
        }
    }
    starts
}

fn line_at<'a>(text: &'a str, starts: &[usize], i: usize) -> &'a str {
    trim_line_end(&text[starts[i]..starts.get(i + 1).copied().unwrap_or(text.len())])
}

fn trim_line_end(slice: &str) -> &str {
    slice
        .strip_suffix('\n')
        .unwrap_or(slice)
        .strip_suffix('\r')
        .unwrap_or(slice)
}

fn is_filter_or_script(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    is_filter_line(&lower) || is_indented_script_tag(&lower)
}

/// Code after `javascript:` / `js:` on the same line (Slim one-liners).
fn inline_filter_code(line: &str) -> Option<(&str, usize)> {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    for prefix in ["javascript:", "js:"] {
        if !lower.starts_with(prefix) {
            continue;
        }
        let after = &trimmed[prefix.len()..];
        let code = after.trim_start();
        if code.is_empty() {
            return None;
        }
        let col = indent_width(line) + prefix.len() + (after.len() - code.len());
        return Some((code, col));
    }
    None
}

fn take_filter_body(
    text: &str,
    starts: &[usize],
    i: &mut usize,
    base: usize,
    filter_line: usize,
    inline: Option<(&str, usize)>,
) -> Option<Fragment> {
    let mut body = String::new();
    let mut start_line = 0usize;
    let mut start_col = 0usize;
    if let Some((code, col)) = inline {
        body.push_str(code);
        body.push('\n');
        start_line = filter_line;
        start_col = col;
    }
    while *i < starts.len() {
        if !append_body_line(
            line_at(text, starts, *i),
            base,
            &mut body,
            &mut start_line,
            &mut start_col,
            *i,
        ) {
            break;
        }
        *i += 1;
    }
    finish_body(body, start_line, start_col)
}

fn append_body_line(
    line: &str,
    base: usize,
    body: &mut String,
    start_line: &mut usize,
    start_col: &mut usize,
    idx: usize,
) -> bool {
    if line.trim().is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        return true;
    }
    if indent_width(line) <= base {
        return false;
    }
    if body.is_empty() {
        *start_line = idx + 1;
        *start_col = indent_width(line);
    }
    body.push_str(strip_indent(line, base));
    body.push('\n');
    true
}

fn finish_body(body: String, start_line: usize, start_col: usize) -> Option<Fragment> {
    if body.trim().is_empty() {
        return None;
    }
    let mut code = body.into_bytes();
    blank_holes(&mut code);
    Some(Fragment {
        lang: Lang::Js,
        code,
        start_line,
        start_col,
        report_root: true,
    })
}

fn is_filter_line(lower: &str) -> bool {
    matches!(lower, "javascript:" | "js:" | ":javascript" | ":js")
        || lower.starts_with("javascript:")
        || lower.starts_with("js:")
}

fn is_indented_script_tag(lower: &str) -> bool {
    let Some(rest) = lower.strip_prefix("script") else {
        return false;
    };
    if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return false;
    }
    !rest.contains("src=") && !rest.contains("src =")
}

fn indent_width(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn strip_indent(line: &str, base: usize) -> &str {
    let mut rest = line;
    for _ in 0..base {
        let mut chars = rest.chars();
        match chars.next() {
            Some(' ' | '\t') => rest = chars.as_str(),
            _ => return line.trim_start(),
        }
    }
    rest
}

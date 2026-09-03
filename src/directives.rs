//! `rubocop:disable` / `enable` directive parsing shared by all languages.

use std::collections::HashSet;

pub struct Directives {
    /// lines carrying a trailing disable naming Metrics/AbcSize (or all cops)
    pub abc_lines: HashSet<usize>,
    /// inclusive ranges where Metrics/AbcSize is disabled
    pub abc_ranges: Vec<(usize, usize)>,
    /// same two, for disables with no cop list (suppress everything)
    pub all_lines: HashSet<usize>,
    pub all_ranges: Vec<(usize, usize)>,
}

impl Directives {
    pub fn suppresses_abc(&self, line: usize) -> bool {
        self.abc_lines.contains(&line)
            || self.all_lines.contains(&line)
            || self
                .abc_ranges
                .iter()
                .chain(self.all_ranges.iter())
                .any(|r| r.0 <= line && line <= r.1)
    }

    pub fn suppresses_all(&self, line: usize) -> bool {
        self.all_lines.contains(&line) || self.all_ranges.iter().any(|r| r.0 <= line && line <= r.1)
    }
}

pub fn cop_names(after: &str) -> Vec<String> {
    after
        .split(',')
        .map(|s| s.trim().trim_start_matches(':').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse(src: &str) -> Directives {
    let mut d = Directives {
        abc_lines: HashSet::new(),
        abc_ranges: Vec::new(),
        all_lines: HashSet::new(),
        all_ranges: Vec::new(),
    };
    let mut pending: Vec<bool> = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line_no = i + 1;
        apply_line(&mut d, &mut pending, line_no, raw);
    }
    d
}

fn classify(comment: &str) -> Directive {
    if let Some(rest) = comment.strip_prefix("rubocop:enable") {
        return Directive::Enable(rest.to_string());
    }
    // `rubocop:todo` behaves exactly like `rubocop:disable`
    if let Some(rest) = comment.strip_prefix("rubocop:todo") {
        return Directive::Disable(rest.to_string());
    }
    // `rubocop:disable-next` is editor-style, not a RuboCop directive
    match comment.strip_prefix("rubocop:disable") {
        Some(rest) if !rest.starts_with('-') => Directive::Disable(rest.to_string()),
        _ => Directive::None,
    }
}

enum Directive {
    Enable(#[allow(dead_code)] String),
    Disable(String),
    None,
}

fn apply_line(d: &mut Directives, pending: &mut Vec<bool>, line_no: usize, raw: &str) {
    // Ruby-style `#` comments and C-family `//` comments; the first
    // marker whose content actually parses as a directive wins.
    let markers = [(raw.find('#'), '#'), (raw.find("//"), '/')];
    for (at, marker) in markers {
        let Some(hash) = at else { continue };
        let Some(comment) = comment_body(raw, hash, marker) else {
            continue;
        };
        match classify(comment) {
            Directive::None => continue,
            Directive::Enable(_) => close_pending(d, pending, line_no),
            Directive::Disable(after) => {
                apply_disable(d, pending, line_no, raw, hash, &after);
            }
        }
        return;
    }
}

/// Comment content at the marker: Ruby strips leading `#` characters;
/// C-family strips `//`, except that a bare `////`-style run stays
/// interesting when it introduces a `rubocop:` word.
fn comment_body<'a>(raw: &'a str, hash: usize, marker: char) -> Option<&'a str> {
    if marker == '#' {
        return Some(raw[hash..].trim_start_matches('#').trim());
    }
    let content = &raw[hash + 2..];
    let trimmed = content.trim_start_matches('/').trim();
    if trimmed.is_empty() && !content.contains("rubocop:") {
        return None;
    }
    Some(trimmed)
}

fn apply_disable(
    d: &mut Directives,
    pending: &mut Vec<bool>,
    line_no: usize,
    raw: &str,
    hash: usize,
    after: &str,
) {
    let names = cop_names(after.trim());

    let relevant = names.is_empty()
        || names
        .iter()
        .any(|n| n == "Metrics/AbcSize" || n == "Metrics");
    let trailing = !raw[..hash].trim().is_empty();
    if trailing {
        push_line(d, line_no, names.is_empty(), relevant);
    } else {
        pending.push(relevant);
        push_range_open(d, line_no, names.is_empty(), relevant);
    }
}

fn push_line(d: &mut Directives, line_no: usize, all: bool, relevant: bool) {
    if all {
        d.all_lines.insert(line_no);
    } else if relevant {
        d.abc_lines.insert(line_no);
    }
}

fn push_range_open(d: &mut Directives, line_no: usize, all: bool, relevant: bool) {
    if all {
        d.all_ranges.push((line_no + 1, usize::MAX));
    } else if relevant {
        d.abc_ranges.push((line_no + 1, usize::MAX));
    }
}

fn close_pending(d: &mut Directives, pending: &mut Vec<bool>, line_no: usize) {
    for targets_abc in pending.drain(..) {
        let ranges = if targets_abc {
            &mut d.abc_ranges
        } else {
            &mut d.all_ranges
        };
        for r in ranges.iter_mut() {
            if r.1 == usize::MAX {
                r.1 = line_no.saturating_sub(1);
            }
        }
    }
}

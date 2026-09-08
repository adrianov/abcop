//! Attribute helpers for `<script>` open tags.

pub(super) fn type_or_lang(attrs: &[u8]) -> Option<String> {
    attr_value(attrs, b"type")
        .or_else(|| attr_value(attrs, b"lang"))
        .map(normalize_attr)
}

fn normalize_attr(v: Vec<u8>) -> String {
    String::from_utf8_lossy(&v)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

pub(super) fn is_js_mime(v: &str) -> bool {
    v.is_empty()
        || v == "module"
        || v == "js"
        || v == "ts"
        || v == "tsx"
        || v.contains("javascript")
        || v.contains("ecmascript")
        || v.contains("typescript")
}

pub(super) fn has_token(attrs: &[u8], name: &[u8]) -> bool {
    let lower = to_ascii_lower(attrs);
    let mut i = 0;
    while let Some(at) = find_subslice(&lower[i..], name) {
        let start = i + at;
        let end = start + name.len();
        if token_bounds(&lower, start, end) {
            return true;
        }
        i = start + 1;
    }
    false
}

fn token_bounds(lower: &[u8], start: usize, end: usize) -> bool {
    let before_ok = start == 0 || lower[start - 1].is_ascii_whitespace();
    let after_ok = end >= lower.len() || lower[end].is_ascii_whitespace() || lower[end] == b'=';
    before_ok && after_ok
}

fn attr_value(attrs: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    let lower = to_ascii_lower(attrs);
    let mut i = 0;
    while let Some(at) = find_subslice(&lower[i..], name) {
        let start = i + at;
        if let Some(v) = value_after_name(attrs, &lower, start, name.len()) {
            return Some(v);
        }
        i = start + 1;
    }
    None
}

fn value_after_name(attrs: &[u8], lower: &[u8], start: usize, namelen: usize) -> Option<Vec<u8>> {
    if start > 0 && !lower[start - 1].is_ascii_whitespace() {
        return None;
    }
    let mut j = skip_ws(lower, start + namelen);
    if j >= lower.len() || lower[j] != b'=' {
        return None;
    }
    j = skip_ws(lower, j + 1);
    Some(read_attr_value(&attrs[j..]))
}

fn skip_ws(buf: &[u8], mut j: usize) -> usize {
    while j < buf.len() && buf[j].is_ascii_whitespace() {
        j += 1;
    }
    j
}

fn read_attr_value(rest: &[u8]) -> Vec<u8> {
    if rest.is_empty() {
        return Vec::new();
    }
    let quote = rest[0];
    if quote == b'"' || quote == b'\'' {
        return quoted_value(rest, quote);
    }
    rest.iter()
        .take_while(|b| !b.is_ascii_whitespace() && **b != b'>')
        .copied()
        .collect()
}

fn quoted_value(rest: &[u8], quote: u8) -> Vec<u8> {
    match rest[1..].iter().position(|&b| b == quote) {
        Some(p) => rest[1..1 + p].to_vec(),
        None => rest[1..].to_vec(),
    }
}

pub(super) fn to_ascii_lower(src: &[u8]) -> Vec<u8> {
    src.iter().map(|b| b.to_ascii_lowercase()).collect()
}

pub(super) fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

pub(super) fn find_byte(hay: &[u8], b: u8) -> Option<usize> {
    hay.iter().position(|&x| x == b)
}

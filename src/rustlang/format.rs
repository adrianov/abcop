//! Format-string implicit captures and identifier-run scanning.

use tree_sitter::Node;

use super::builder::Builder;

impl<'m> Builder<'m> {
    /// Rust format strings implicitly capture named arguments:
    /// `format!("{msg}")` reads `msg`. Record those as variable reads.
    pub(super) fn record_format_captures(&mut self, literal: Node, scope: usize) {
        let text = self.text(literal).to_string();
        let base = literal.start_byte();
        for (start, end) in format_holes(&text) {
            let content = &text[start + 1..end];
            let content_abs = base + start + 1;
            self.capture_idents(content, content_abs, scope);
        }
    }

    fn capture_idents(&mut self, content: &str, content_abs_start: usize, scope: usize) {
        for (start, name) in ident_runs(content) {
            let abs = content_abs_start + start;
            if !name.starts_with('_') && self.lookup(scope, abs, name).is_some() {
                self.record_read(scope, name, abs);
            }
        }
    }
}

/// Byte ranges `(start, end)` of `{...}` holes inside a format string,
/// with `{{`/`}}` escapes skipped. Scanning stops at an unterminated hole.
fn format_holes(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut holes = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                i += 2;
            }
            b'{' => {
                let Some(j) = closing_brace(bytes, i) else {
                    return holes;
                };
                holes.push((i, j));
                i = j + 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    holes
}

/// Index of the `}` closing the hole opened at `open`, or `None` when the
/// hole is unterminated.
fn closing_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut j = open + 1;
    while j < bytes.len() && bytes[j] != b'}' {
        j += 1;
    }
    (j < bytes.len()).then_some(j)
}

/// Byte spans of identifier-shaped runs (`[A-Za-z_][A-Za-z0-9_]*`):
/// `(offset_in_s, &s)`.
pub(super) fn ident_runs(s: &str) -> Vec<(usize, &str)> {
    let bytes = s.as_bytes();
    let mut runs = Vec::new();
    let mut k = 0usize;
    while k < bytes.len() {
        if bytes[k] == b'_' || bytes[k].is_ascii_alphabetic() {
            let start = k;
            while k < bytes.len() && (bytes[k] == b'_' || bytes[k].is_ascii_alphanumeric()) {
                k += 1;
            }
            runs.push((start, &s[start..k]));
        } else {
            k += 1;
        }
    }
    runs
}

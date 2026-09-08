//! Blank template interpolations inside extracted JS so tree-sitter can parse
//! the surrounding script. Replacements keep the same byte length so fragment
//! line/column remap stays aligned with the host file.

/// Replace common template holes with spaces of equal length (newlines kept).
pub(super) fn blank_holes(code: &mut [u8]) {
    blank_pairs(code, b"<%", b"%>");
    blank_pairs(code, b"{%", b"%}");
    blank_pairs(code, b"{{", b"}}");
    blank_pairs(code, b"{#", b"#}");
}

fn blank_pairs(code: &mut [u8], open: &[u8], close: &[u8]) {
    let mut i = 0;
    while i + open.len() <= code.len() {
        if !code[i..].starts_with(open) {
            i += 1;
            continue;
        }
        let start = i;
        i += open.len();
        while i + close.len() <= code.len() {
            if code[i..].starts_with(close) {
                i += close.len();
                space_out(&mut code[start..i]);
                break;
            }
            i += 1;
        }
    }
}

fn space_out(slice: &mut [u8]) {
    for b in slice {
        if *b != b'\n' && *b != b'\r' {
            *b = b' ';
        }
    }
}

//! Debug helper: pretty-print a parsed syntax tree.

use std::fs;

use crate::paths::lang_for;

pub fn dump_tree(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let src = fs::read(path)?;

    let tree = crate::paths::parse_file_lang(&src, lang_for(std::path::Path::new(path)))
        .ok_or("parse failed")?;
    rec(&mut tree.walk(), 0, &src);
    Ok(())
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn shorten(text: &str) -> String {
    if text.len() <= 60 {
        text.replace('\n', "\\n")
    } else {
        format!("{}…", text[..60].replace('\n', "\\n"))
    }
}

fn rec(cursor: &mut tree_sitter::TreeCursor, depth: usize, src: &[u8]) {
    loop {
        emit(cursor, src, depth);
        if cursor.goto_first_child() {
            rec(cursor, depth + 1, src);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn emit(cursor: &mut tree_sitter::TreeCursor, src: &[u8], depth: usize) {
    let n = cursor.node();
    let field = cursor.field_name().unwrap_or("");
    let prefix = if field.is_empty() {
        String::new()
    } else {
        format!("@{field}: ")
    };
    println!(
        "{ind}{prefix}{kind} [{row}:{col}] {text}",
        ind = indent(depth),
        kind = n.kind(),
        row = n.start_position().row + 1,
        col = n.start_position().column,
        text = shorten(n.utf8_text(src).unwrap_or(""))
    );
}

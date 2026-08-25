//! Language detection and file-tree collection.

use tree_sitter::Parser;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ruby,
    Rust,
}

pub fn lang_for(path: &std::path::Path) -> Lang {
    let rust_ext = path.extension().and_then(|e| e.to_str()) == Some("rs");
    if rust_ext {
        Lang::Rust
    } else {
        Lang::Ruby
    }
}

pub fn parse_file_lang(src: &[u8], lang: Lang) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let ts_lang = match lang {
        Lang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
    };
    parser.set_language(&ts_lang).ok()?;
    parser.parse(src, None)
}

const CODE_EXTS: [&str; 5] = ["rb", "rake", "ru", "gemspec", "rs"];
const CODE_NAMES: [&str; 6] = [
    "Gemfile",
    "Rakefile",
    "Capfile",
    "Brewfile",
    "Podfile",
    "Fastfile",
];

pub fn is_code_path(p: &std::path::Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) => CODE_EXTS.contains(&ext),
        None => p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| CODE_NAMES.contains(&n))
            .unwrap_or(false),
    }
}

pub fn collect_files(paths: &[String]) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for raw in paths {
        collect_one(raw, &mut files);
    }
    files.sort();
    files.dedup();
    files
}

fn collect_one(raw: &str, files: &mut Vec<std::path::PathBuf>) {
    let p = std::path::Path::new(raw);
    if p.is_file() {
        files.push(p.to_path_buf());
        return;
    }
    let walker = ignore::WalkBuilder::new(p).build();
    for entry in walker.flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && is_code_path(entry.path())
        {
            files.push(entry.into_path());
        }
    }
}

//! Language detection and code-path classification.
//!
//! Directory traversal lives in `walker`; default-skip policy in `skip`.

use tree_sitter::Parser;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ruby,
    Rust,
    Js,
    Ts,
    Tsx,
    C,
    Cpp,
    ObjC,
    Swift,
}

impl Lang {
    /// True for the C-family backends scored by `clike` (ABC only).
    pub fn is_clike(self) -> bool {
        matches!(
            self,
            Lang::Js | Lang::Ts | Lang::Tsx | Lang::C | Lang::Cpp
                | Lang::ObjC | Lang::Swift
        )
    }
}

pub fn lang_for(path: &std::path::Path) -> Lang {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Lang::Rust,
        Some("js" | "mjs" | "cjs" | "jsx") => Lang::Js,
        Some("ts" | "mts" | "cts") => Lang::Ts,
        Some("tsx") => Lang::Tsx,
        Some("c" | "h") => Lang::C,
        Some("cc" | "cpp" | "cxx" | "hpp" | "hxx" | "hh") => Lang::Cpp,
        Some("m" | "mm") => Lang::ObjC,
        Some("swift") => Lang::Swift,
        // Gemfile-style names and anything else text-shaped stay Ruby,
        // whose scorer is a no-op for non-Ruby content.
        _ => Lang::Ruby,
    }
}

pub fn parse_file_lang(src: &[u8], lang: Lang) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let ts_lang = match lang {
        Lang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Js => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::C => tree_sitter_c::LANGUAGE.into(),
        Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Lang::ObjC => tree_sitter_objc::LANGUAGE.into(),
        Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
    };
    parser.set_language(&ts_lang).ok()?;
    parser.parse(src, None)
}

const CODE_EXTS: [&str; 24] = [
    "rb", "rake", "ru", "gemspec", "rs", //
    "js", "mjs", "cjs", "jsx", "ts", "tsx", "mts", "cts", //
    "c", "h", "cc", "cpp", "cxx", "hpp", "hxx", "hh", //
    "m", "mm", "swift",
];

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

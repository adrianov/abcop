//! Language detection and code-path classification.
//!
//! Directory traversal lives in `walker`; default-skip policy in `skip`.

use tree_sitter::Parser;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Ruby,
    Rust,
    Py,
    Go,
    Java,
    Php,
    CSharp,
    Solidity,
    Dart,
    Zig,
    Hs,
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
            Lang::Js | Lang::Ts | Lang::Tsx | Lang::C | Lang::Cpp | Lang::ObjC | Lang::Swift
        )
    }
}

/// Ruby source extensions (and Gemfile-style basenames via [`CODE_NAMES`]).
const RUBY_EXTS: &[&str] = &["rb", "rake", "ru", "gemspec"];

/// Extension → language. `.h` rides the C++ grammar: the C grammar
/// misreads `class` / `namespace` bodies as function definitions and
/// then NeverUsed fires on every member.
const EXT_LANG: &[(&[&str], Lang)] = &[
    (&["rs"], Lang::Rust),
    (&["js", "mjs", "cjs", "jsx"], Lang::Js),
    (&["ts", "mts", "cts"], Lang::Ts),
    (&["tsx"], Lang::Tsx),
    (&["c"], Lang::C),
    (&["h", "cc", "cpp", "cxx", "hpp", "hxx", "hh"], Lang::Cpp),
    (&["m", "mm"], Lang::ObjC),
    (&["swift"], Lang::Swift),
    (&["py", "pyi", "pyw"], Lang::Py),
    (&["go"], Lang::Go),
    (&["php"], Lang::Php),
    (&["java"], Lang::Java),
    (&["cs"], Lang::CSharp),
    (&["sol"], Lang::Solidity),
    (&["dart"], Lang::Dart),
    (&["zig"], Lang::Zig),
    (&["hs", "lhs"], Lang::Hs),
];

pub fn lang_for(path: &std::path::Path) -> Lang {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Lang::Ruby;
    };
    for (exts, lang) in EXT_LANG {
        if exts.contains(&ext) {
            return *lang;
        }
    }
    // Gemfile-style names and anything else text-shaped stay Ruby,
    // whose scorer is a no-op for non-Ruby content.
    Lang::Ruby
}

pub fn parse_file_lang(src: &[u8], lang: Lang) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(&grammar_of(lang)?).ok()?;
    parser.parse(src, None)
}

/// Grammar of a language. The split mirrors the dispatch architecture:
/// the `clike` scanner family versus the standalone per-language
/// backends -- each side stays a short, flat table.
fn grammar_of(lang: Lang) -> Option<tree_sitter::Language> {
    if lang.is_clike() {
        clike_grammar(lang)
    } else {
        standalone_grammar(lang)
    }
}

fn clike_grammar(lang: Lang) -> Option<tree_sitter::Language> {
    let ts_lang = match lang {
        Lang::Js => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Ts => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::C => tree_sitter_c::LANGUAGE.into(),
        Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Lang::ObjC => tree_sitter_objc::LANGUAGE.into(),
        Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
        _ => return None,
    };
    Some(ts_lang)
}

fn standalone_grammar(lang: Lang) -> Option<tree_sitter::Language> {
    let ts_lang = match lang {
        Lang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::Py => tree_sitter_python::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::Php => tree_sitter_php::LANGUAGE_PHP_ONLY.into(),
        Lang::Java => tree_sitter_java::LANGUAGE.into(),
        Lang::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Lang::Solidity => tree_sitter_solidity::LANGUAGE.into(),
        Lang::Dart => tree_sitter_dart::LANGUAGE.into(),
        Lang::Zig => tree_sitter_zig::LANGUAGE.into(),
        Lang::Hs => tree_sitter_haskell::LANGUAGE.into(),
        _ => return None,
    };
    Some(ts_lang)
}

const CODE_NAMES: [&str; 6] = [
    "Gemfile", "Rakefile", "Capfile", "Brewfile", "Podfile", "Fastfile",
];

fn is_code_ext(ext: &str) -> bool {
    RUBY_EXTS.contains(&ext) || EXT_LANG.iter().any(|(exts, _)| exts.contains(&ext))
}

pub fn is_code_path(p: &std::path::Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) => is_code_ext(ext),
        None => p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| CODE_NAMES.contains(&n))
            .unwrap_or(false),
    }
}

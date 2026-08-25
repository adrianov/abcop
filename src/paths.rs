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

/// Walk all directory roots in parallel, one worker thread per available
/// CPU (traversal is syscall/IO bound), collecting matching code files.
/// Single-file arguments bypass the walker. Output is sorted and deduped so
/// downstream results stay deterministic regardless of discovery order.
pub fn collect_files(paths: &[String]) -> Vec<std::path::PathBuf> {
    let mut direct = Vec::new();
    let mut roots = Vec::new();
    for raw in paths {
        let p = std::path::Path::new(raw);
        if p.is_file() {
            direct.push(p.to_path_buf());
        } else if p.exists() {
            roots.push(p.to_path_buf());
        }
    }

    let threads =
        std::thread::available_parallelism().map_or(1, |n| n.get());

    let discovered = std::sync::Mutex::new(Vec::new());
    if !roots.is_empty() {
        let mut builder = ignore::WalkBuilder::new(&roots[0]);
        for r in &roots[1..] {
            builder.add(r);
        }
        builder.threads(threads);
        let sink = &discovered;
        let mut collector = CollectorBuilder { sink };
        builder.build_parallel().visit(&mut collector);
    }

    let mut files = direct;
    files.extend(discovered.into_inner().unwrap_or_default());
    files.sort();
    files.dedup();
    files
}

/// Shared-sink parallel visitor: one per worker thread.
struct CollectorBuilder<'s> {
    sink: &'s std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for CollectorBuilder<'s> {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(Collector { sink: self.sink })
    }
}

struct Collector<'s> {
    sink: &'s std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl ignore::ParallelVisitor for Collector<'_> {
    fn visit(
        &mut self,
        entry: Result<ignore::DirEntry, ignore::Error>,
    ) -> ignore::WalkState {
        if let Ok(entry) = entry
            && entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && is_code_path(entry.path())
            && let Ok(mut sink) = self.sink.lock()
        {
            sink.push(entry.into_path());
        }
        ignore::WalkState::Continue
    }
}

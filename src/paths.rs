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
/// A discovered file plus its depth below the walked root.
#[derive(Debug)]
struct Found {
    path: std::path::PathBuf,
    depth: usize,
}

/// Walk directory roots breadth-first and return code files ordered by
/// depth (shallowest first), then by parent directory, extension and file
/// name. Explicit file arguments come first at depth 0.
///
/// The parallel walk itself is unordered; the deterministic order is
/// produced by the final multi-key sort over the recorded depths.
pub fn collect_files(paths: &[String]) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut roots = Vec::new();
    for raw in paths {
        let p = std::path::Path::new(raw);
        if p.is_file() {
            found.push(Found {
                path: p.to_path_buf(),
                depth: 0,
            });
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

    // Merge explicit file arguments (depth 0) with parallel discovery.
    let mut all: Vec<Found> = found;
    all.extend(discovered.into_inner().unwrap_or_default());

    // Stable multi-key sort turns the unordered parallel discovery into a
    // deterministic breadth-first listing: shallowest entries first, files
    // grouped per directory, then by extension and file name.
    all.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.parent().cmp(&b.parent()))
            .then_with(|| a.ext_key().cmp(&b.ext_key()))
            .then_with(|| a.name_key().cmp(&b.name_key()))
    });
    all.dedup_by(|a, b| a.path == b.path);

    all.into_iter().map(|f| f.path).collect()
}

trait PathKey {
    fn parent(&self) -> String;
    fn ext_key(&self) -> String;
    fn name_key(&self) -> String;
}

impl PathKey for Found {
    fn parent(&self) -> String {
        self.path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }
    fn ext_key(&self) -> String {
        self.path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }
    fn name_key(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }
}

/// Shared-sink parallel visitor: one per worker thread.
struct CollectorBuilder<'s> {
    sink: &'s std::sync::Mutex<Vec<Found>>,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for CollectorBuilder<'s> {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(Collector { sink: self.sink })
    }
}

struct Collector<'s> {
    sink: &'s std::sync::Mutex<Vec<Found>>,
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
            let depth = entry.depth();
            sink.push(Found {
                path: entry.into_path(),
                depth,
            });
        }
        ignore::WalkState::Continue
    }
}

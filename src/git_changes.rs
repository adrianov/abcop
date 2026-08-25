//! Git working-tree change detection for `--changed` mode.
//!
//! Semantics mirror refactor_gpt quality gates: compare the working tree
//! against a base ref (`HEAD` by default) with `git diff -U0 -W`: the
//! function-context option expands every hunk to the full enclosing
//! function, so a hunk range IS a touched function body. New-side line
//! numbers are collected from `@@` headers, plus untracked files
//! (`ls-files --others --exclude-standard`), which count as fully changed.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// Which lines of a changed file were touched.
#[derive(Debug)]
pub enum Lines {
    /// untracked/new file: every line counts
    All,
    Ranges(BTreeSet<usize>),
}

#[derive(Debug)]
pub struct Changeset {
    pub root: String,
    pub files: BTreeMap<String, Lines>,
}

pub fn git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

impl Changeset {
    pub fn load(base: &str) -> Result<Changeset, String> {
        let root = git(&["rev-parse", "--show-toplevel"])?
            .trim()
            .replace('\\', "/");
        let mut files = BTreeMap::new();
        parse_diff(&git(&["diff", "-U0", base])?, &mut files);
        add_untracked(&mut files);
        Ok(Changeset { root, files })
    }

    pub fn line_selected(&self, rel: &str, line: usize) -> bool {
        match self.files.get(rel) {
            None => false,
            Some(Lines::All) => true,
            Some(Lines::Ranges(set)) => set.contains(&line),
        }
    }

    /// True when any changed line falls inside `[start, end]`.
    pub fn span_selected(&self, rel: &str, start: usize, end: usize) -> bool {
        match self.files.get(rel) {
            None => false,
            Some(Lines::All) => true,
            Some(Lines::Ranges(set)) => set
                .range(start..=end)
                .next()
                .map(|l| *l <= end)
                .unwrap_or(false),
        }
    }

    /// Repo-relative path of an absolute or already-relative path.
    pub fn rel_of<'a>(&'a self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("{}/", self.root);
        path.strip_prefix(&prefix)
            .or_else(|| (path == self.root).then_some(""))
    }

    /// Changed code files that still exist on disk, as absolute paths.
    pub fn code_files(&self) -> Vec<std::path::PathBuf> {
        use std::path::Path;
        self.files
            .keys()
            .filter(|k| crate::paths::is_code_path(std::path::Path::new(k)))
            .map(|k| Path::new(&self.root).join(k))
            .filter(|p| p.exists())
            .collect()
    }

}

fn add_untracked(files: &mut BTreeMap<String, Lines>) {
    let untracked =
        git(&["ls-files", "--others", "--exclude-standard", "-z"]).unwrap_or_default();
    for f in untracked.split('\0').filter(|s| !s.is_empty()) {
        files.insert(normalize(f), Lines::All);
    }
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

/// Parse a unified diff, collecting new-side line numbers per file.
///
/// State machine over the four header kinds we care about; handles new files
/// (`--- /dev/null` followed by `+++ b/path`), deletions (`+++ /dev/null`,
/// whose hunks are dropped) and multi-file diffs.
enum LineKind {
    DiffStart,
    OldHeader,
    NewHeader(String),
    DevNullTarget,
    Hunk(usize, usize),
    Other,
}

fn classify(line: &str) -> LineKind {
    if let Some(rest) = line.strip_prefix("@@") {
        return hunk(rest);
    }
    if line.starts_with("diff --git ") {
        return LineKind::DiffStart;
    }
    if line.starts_with("--- ") {
        return LineKind::OldHeader;
    }
    if let Some(path) = line.strip_prefix("+++ b/") {
        return LineKind::NewHeader(normalize(path.trim()));
    }
    if line.starts_with("+++ ") || line.starts_with("--- ") {
        return LineKind::DevNullTarget;
    }
    LineKind::Other
}

fn hunk(rest: &str) -> LineKind {
    let Some((start, count)) = hunk_spec(rest) else {
        return LineKind::Other;
    };
    if start == 0 {
        // a new-side range starting at zero carries no reportable lines
        return LineKind::Other;
    }
    LineKind::Hunk(start, count)
}

/// Parse the `+start[,count]` part of a hunk header.
fn hunk_spec(rest: &str) -> Option<(usize, usize)> {
    let plus_idx = rest.find('+')?;
    let tail = &rest[plus_idx + 1..];
    let digits_end = tail
        .find(|c: char| !(c.is_ascii_digit() || c == ','))
        .unwrap_or(tail.len());
    let mut parts = tail[..digits_end].split(',');
    let start = parts.next()?.parse::<usize>().ok()?;
    let count = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    Some((start, count))
}

/// Parse a unified diff (already widened with -W by the caller), collecting
/// new-side line numbers per file.
///
/// Handles new files (`--- /dev/null` then `+++ b/path`), deletions
/// (`+++ /dev/null`: hunks are dropped) and multi-file diffs.
pub(crate) fn parse_diff(diff: &str, files: &mut BTreeMap<String, Lines>) {
    #[derive(Default)]
    struct Cur {
        path: Option<String>,
        set: BTreeSet<usize>,
    }

    fn flush(files: &mut BTreeMap<String, Lines>, cur: &mut Option<Cur>) {
        if let Some(c) = cur.take()
            && let Some(path) = c.path
        {
            files.insert(path, Lines::Ranges(c.set));
        }
    }

    let mut cur: Option<Cur> = None;
    for line in diff.lines() {
        match classify(line) {
            LineKind::DiffStart | LineKind::OldHeader => flush(files, &mut cur),
            LineKind::NewHeader(path) => {
                cur.get_or_insert_with(Cur::default).path = Some(path);
            }
            LineKind::Hunk(start, count) => {
                if let Some(c) = cur.as_mut() {
                    c.set.extend(start..start + count.max(1));
                }
            }
            LineKind::DevNullTarget | LineKind::Other => {}
        }
    }
    flush(files, &mut cur);
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -2,0 +3,1 @@
+fn added()
@@ -10 +11 @@
-old x
+new x
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-dropped
diff --git a/b/new.rs b/b/new.rs
new file mode 100644
--- /dev/null
+++ b/b/new.rs
@@ -0,0 +1,2 @@
+let a = 1;
+let b = 2;
";

    #[test]
    fn parses_files_and_hunk_ranges() {
        let mut files = BTreeMap::new();
        parse_diff(DIFF, &mut files);
        assert_eq!(files.len(), 2);

        let a = files.get("a.rs").unwrap();
        assert!(matches!(a, Lines::Ranges(_)));
        assert!(a.line_selected(3));
        assert!(a.line_selected(11));
        assert!(!a.line_selected(4));

        let newf = files.get("b/new.rs").unwrap();
        assert!(newf.line_selected(1));
        assert!(newf.line_selected(2));
        assert!(!newf.line_selected(3));
    }

    #[test]
    fn deleted_files_are_not_tracked() {
        let mut files = BTreeMap::new();
        parse_diff(DIFF, &mut files);
        assert!(!files.contains_key("gone.rs"));
    }

    #[test]
    fn span_selection_semantics() {
        let mut set = BTreeSet::new();
        set.insert(15);
        let cs = Lines::Ranges(set);
        let selected = match &cs {
            Lines::All => true,
            Lines::Ranges(r) => r.range(10..=20).next().is_some(),
        };
        assert!(selected);

        let none = Lines::Ranges(BTreeSet::from([5]));
        let outside = match &none {
            Lines::Ranges(r) => r.range(10..=20).next().is_none(),
            _ => true,
        };
        assert!(outside);
    }

    impl Lines {
        fn line_selected(&self, line: usize) -> bool {
            match self {
                Lines::All => true,
                Lines::Ranges(r) => r.contains(&line),
            }
        }
    }
}

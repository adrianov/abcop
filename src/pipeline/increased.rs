//! Edit filter against HEAD.
//!
//! AbcSize is reported only when the working-tree score is strictly
//! higher than the same function in HEAD. UsedOnce and NeverUsed are
//! reported only inside a function whose text changed. Same names pair
//! in source order, so an earlier copy cannot consume a later copy's
//! baseline. A name missing from HEAD counts as new. No HEAD blob
//! (untracked file, or not a git checkout) keeps the absolute hits.
//! Identical bytes clear those three rules. ModuleAbcSize is left alone.
//!
//! Pairing uses the unit inventory captured during the file's own
//! analysis (and cached with it). The HEAD inventory is cached by blob
//! id, so a later scan does not parse either side again.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use super::baseline::{Head, head_units, load_head};
use crate::abc::Unit;
use crate::output::FileResult;

/// Drop edit-scoped hits when `enabled`: AbcSize that did not rise, and
/// locals that sit in a function whose text matches HEAD.
pub(super) fn apply(
    enabled: bool,
    path: &Path,
    src: &[u8],
    cache: Option<&crate::cache::Cache>,
    r: &mut FileResult,
) {
    if !enabled || (r.abc.is_empty() && r.used_once.is_empty() && r.never_used.is_empty()) {
        return;
    }
    let Some(head) = load_head(path, src) else {
        return;
    };
    let Head::Diff { oid, bytes } = head else {
        r.abc.clear();
        r.used_once.clear();
        r.never_used.clear();
        return;
    };
    let base = head_units(cache, path, oid, &bytes);
    let current = std::mem::take(&mut r.units);
    keep_risen(&current, &base, r);
    keep_edited_locals(src, &bytes, &current, &base, r);
}

fn keep_risen(current: &[Unit], base: &[Unit], r: &mut FileResult) {
    if r.abc.is_empty() {
        return;
    }
    let priors = risen_priors(current, base);
    r.abc.retain(|o| priors.contains_key(&o.line));
    for o in &mut r.abc {
        o.prior = priors[&o.line];
    }
}

/// Locals outside every function stay. Locals inside a function stay only
/// when that function's text differs from HEAD or the function is new.
fn keep_edited_locals(
    src: &[u8],
    head: &[u8],
    current: &[Unit],
    base: &[Unit],
    r: &mut FileResult,
) {
    if r.used_once.is_empty() && r.never_used.is_empty() {
        return;
    }
    let edited = edited_spans(src, head, current, base);
    r.used_once.retain(|o| local_kept(o.line, current, &edited));
    r.never_used
        .retain(|o| local_kept(o.line, current, &edited));
}

fn local_kept(line: usize, units: &[Unit], edited: &[(usize, usize)]) -> bool {
    !units.iter().any(|o| line >= o.line && line <= o.end_line)
        || edited
            .iter()
            .any(|&(start, end)| line >= start && line <= end)
}

fn edited_spans(src: &[u8], head: &[u8], current: &[Unit], base: &[Unit]) -> Vec<(usize, usize)> {
    let mut prior = span_queue(base);
    let mut spans = Vec::new();
    for o in current {
        if span_changed(
            src,
            head,
            o,
            prior.get_mut(o.name.as_str()).and_then(|q| q.pop_front()),
        ) {
            spans.push((o.line, o.end_line));
        }
    }
    spans
}

fn span_changed(src: &[u8], head: &[u8], unit: &Unit, old: Option<&Unit>) -> bool {
    let Some(old) = old else {
        return true;
    };
    if unit.end_byte > unit.start_byte && old.end_byte > old.start_byte {
        return src.get(unit.start_byte..unit.end_byte) != head.get(old.start_byte..old.end_byte);
    }
    line_text(src, unit.line, unit.end_line) != line_text(head, old.line, old.end_line)
}

fn span_queue(units: &[Unit]) -> HashMap<&str, VecDeque<&Unit>> {
    let mut prior: HashMap<&str, VecDeque<&Unit>> = HashMap::new();
    for o in units {
        prior.entry(o.name.as_str()).or_default().push_back(o);
    }
    prior
}

/// Line text for a span that is not a byte slice of the host (a
/// reconstructed template block). The common path compares byte slices.
fn line_text(src: &[u8], start: usize, end: usize) -> String {
    if start == 0 || end < start {
        return String::new();
    }
    String::from_utf8_lossy(src)
        .lines()
        .skip(start - 1)
        .take(end - start + 1)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Start line → HEAD score. `None` means the function is new.
fn risen_priors(current: &[Unit], base: &[Unit]) -> HashMap<usize, Option<f64>> {
    let mut prior: HashMap<&str, VecDeque<f64>> = HashMap::new();
    for o in base {
        prior.entry(o.name.as_str()).or_default().push_back(o.score);
    }
    let mut risen = HashMap::new();
    for o in current {
        match prior.get_mut(o.name.as_str()).and_then(|q| q.pop_front()) {
            Some(score) if o.score > score => {
                risen.insert(o.line, Some(score));
            }
            None => {
                risen.insert(o.line, None);
            }
            Some(_) => {}
        }
    }
    risen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abc::Limits;
    use crate::modulesize;
    use crate::output::FileResult;
    use std::collections::HashSet;

    fn at(name: &str, line: usize, score: f64) -> Unit {
        Unit {
            name: name.into(),
            line,
            end_line: line,
            start_byte: 0,
            end_byte: 0,
            score,
        }
    }

    fn limits(increased: bool) -> Limits {
        Limits {
            method: 17.0,
            module: modulesize::MAX_ABC,
            increased,
        }
    }

    fn method(name: &str, prefix: &str, n: usize) -> String {
        let body: String = (1..=n).map(|i| format!("  {prefix}{i} = {i}\n")).collect();
        format!("def {name}\n{body}end\n")
    }

    fn write(dir: &Path, body: &str) {
        std::fs::write(dir.join("a.rb"), body).unwrap();
    }

    fn scan(dir: &Path, increased: bool) -> FileResult {
        super::super::analyze_one(&dir.join("a.rb"), None, limits(increased), None, None)
    }

    /// Units from a used-once-only walk, before the edit filter takes them.
    fn units_of(name: &str, src: &str) -> Vec<Unit> {
        let path = Path::new(name);
        let mut r = super::super::blank_with(path);
        assert!(super::super::analyze_src_into(
            &mut r,
            path,
            src.as_bytes(),
            Some("used-once"),
            limits(true),
        ));
        assert!(r.abc.is_empty());
        r.units
    }

    fn commit_ruby(tag: &str, body: &str) -> std::path::PathBuf {
        let dir = crate::test_repo::temp_dir(tag);
        crate::test_repo::seed_repo_with_base_commit(&dir);
        write(&dir, body);
        crate::test_repo::commit_all(&dir, "base");
        dir
    }

    fn repo_with_legacy(tag: &str) -> std::path::PathBuf {
        commit_ruby(tag, &method("legacy", "a", 20))
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_name_consumes_baseline_in_order() {
        let base = vec![at("foo", 1, 10.0), at("foo", 30, 40.0), at("bar", 50, 20.0)];
        let current = vec![
            at("foo", 1, 12.0),
            at("foo", 30, 40.0),
            at("bar", 60, 20.0),
            at("baz", 70, 18.0),
        ];
        let risen = risen_priors(&current, &base);
        assert_eq!(
            risen.keys().copied().collect::<HashSet<_>>(),
            HashSet::from([1, 70])
        );
        assert_eq!(risen[&1], Some(10.0));
        assert_eq!(risen[&70], None);
    }

    #[test]
    fn under_ceiling_sibling_does_not_steal_a_later_slot() {
        let base = vec![at("foo", 1, 10.0), at("foo", 40, 30.0)];
        let current = vec![at("foo", 1, 12.0), at("foo", 40, 30.0)];
        assert!(!risen_priors(&current, &base).contains_key(&40));
    }

    #[test]
    fn committed_over_ceiling_stays_quiet() {
        let dir = repo_with_legacy("abc_quiet");
        assert!(
            scan(&dir, true).abc.is_empty(),
            "committed legacy stays quiet"
        );
        assert_eq!(scan(&dir, false).abc.len(), 1, "absolute still flags it");
        cleanup(&dir);
    }

    #[test]
    fn edit_that_keeps_the_score_stays_quiet() {
        let dir = repo_with_legacy("abc_same");
        write(&dir, &method("legacy", "a", 20).replace("a1 = 1", "a1 = 2"));
        assert!(scan(&dir, true).abc.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn score_rise_records_the_head_score() {
        let dir = repo_with_legacy("abc_rose");
        write(&dir, &method("legacy", "a", 21));
        let rose = scan(&dir, true);
        assert_eq!(rose.abc.len(), 1);
        assert_eq!(rose.abc[0].name, "legacy");
        assert!(rose.abc[0].prior.is_some_and(|was| rose.abc[0].score > was));
        cleanup(&dir);
    }

    #[test]
    fn lower_score_stays_quiet_while_still_over_ceiling() {
        let dir = repo_with_legacy("abc_down");
        write(&dir, &method("legacy", "a", 18));
        assert!(scan(&dir, true).abc.is_empty());
        assert!(scan(&dir, false).abc.iter().any(|o| o.score > 17.0));
        cleanup(&dir);
    }

    #[test]
    fn new_over_ceiling_method_reports() {
        let dir = repo_with_legacy("abc_new");
        write(
            &dir,
            &format!("{}{}", method("legacy", "a", 20), method("fresh", "b", 20)),
        );
        assert_eq!(
            scan(&dir, true)
                .abc
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            vec!["fresh"]
        );
        cleanup(&dir);
    }

    #[test]
    fn used_once_still_reports_when_abc_does_not_rise() {
        let dir = repo_with_legacy("abc_used");
        write(
            &dir,
            &format!(
                "{}def extra\n  tmp = 1\n  tmp\nend\n",
                method("legacy", "a", 20)
            ),
        );
        let hit = scan(&dir, true);
        assert!(hit.abc.is_empty());
        assert!(hit.used_once.iter().any(|o| o.name == "tmp"));
        cleanup(&dir);
    }

    #[test]
    fn locals_in_an_untouched_function_stay_quiet() {
        let dir = commit_ruby(
            "abc_locals",
            "def legacy\n  stale = 1\n  tmp = 1\n  tmp\nend\ndef other\n  n = 1\nend\n",
        );
        write(
            &dir,
            "def legacy\n  stale = 1\n  tmp = 1\n  tmp\nend\ndef other\n  n = 1\n  m = 2\nend\n",
        );
        let hit = scan(&dir, true);
        assert!(hit.never_used.iter().all(|o| o.name != "stale"));
        assert!(hit.used_once.iter().all(|o| o.name != "tmp"));
        assert!(hit.never_used.iter().any(|o| o.name == "m"));
        let absolute = scan(&dir, false);
        assert!(absolute.never_used.iter().any(|o| o.name == "stale"));
        assert!(absolute.used_once.iter().any(|o| o.name == "tmp"));
        cleanup(&dir);
    }

    #[test]
    fn disabled_legacy_method_locals_stay_quiet() {
        let dir = commit_ruby(
            "abc_disable",
            "def legacy # rubocop:disable Metrics/AbcSize\n  stale = 1\nend\ndef other\n  n = 1\nend\n",
        );
        write(
            &dir,
            "def legacy # rubocop:disable Metrics/AbcSize\n  stale = 1\nend\ndef other\n  n = 1\n  m = 2\nend\n",
        );
        let hit = scan(&dir, true);
        assert!(hit.never_used.iter().all(|o| o.name != "stale"));
        assert!(hit.never_used.iter().any(|o| o.name == "m"));
        cleanup(&dir);
    }

    #[test]
    fn used_once_only_skips_untouched_functions() {
        let dir = commit_ruby(
            "abc_only_used",
            "def legacy\n  tmp = 1\n  tmp\nend\ndef other\n  n = 1\nend\n",
        );
        write(
            &dir,
            "def legacy\n  tmp = 1\n  tmp\nend\ndef other\n  n = 1\n  k = 2\n  k\nend\n",
        );
        let hit = super::super::analyze_one(
            &dir.join("a.rb"),
            Some("used-once"),
            limits(true),
            None,
            None,
        );
        assert!(hit.used_once.iter().all(|o| o.name != "tmp"));
        assert!(hit.used_once.iter().any(|o| o.name == "k"));
        cleanup(&dir);
    }

    #[test]
    fn used_once_only_records_units() {
        let ruby = units_of("a.rb", "def legacy\n  tmp = 1\n  tmp\nend\n");
        assert!(ruby.iter().any(|u| u.name == "legacy"));
        let host = units_of(
            "page.html",
            "<script>\nfunction boot() {\n  const k = 1;\n  k;\n}\n</script>\n",
        );
        assert!(host.iter().any(|u| u.name == "boot"));
    }

    #[test]
    fn untracked_over_ceiling_method_reports() {
        let dir = repo_with_legacy("abc_untracked");
        let added = dir.join("b.rb");
        std::fs::write(&added, method("added", "c", 20)).unwrap();
        let hit = super::super::analyze_one(&added, Some("abc"), limits(true), None, None);
        assert_eq!(hit.abc.len(), 1);
        assert_eq!(hit.abc[0].name, "added");
        cleanup(&dir);
    }

    #[test]
    fn redefined_name_compares_each_copy() {
        let dir = crate::test_repo::temp_dir("abc_redef");
        crate::test_repo::seed_repo_with_base_commit(&dir);
        write(
            &dir,
            &format!("{}{}", method("foo", "a", 1), method("foo", "b", 20)),
        );
        crate::test_repo::commit_all(&dir, "two foos");
        write(
            &dir,
            &format!("{}{}", method("foo", "a", 3), method("foo", "b", 20)),
        );
        assert!(
            scan(&dir, true).abc.is_empty(),
            "the untouched copy must not inherit the small copy's baseline"
        );
        cleanup(&dir);
    }

    #[test]
    fn cached_scan_keeps_the_rise() {
        let dir = repo_with_legacy("abc_cache");
        write(&dir, &method("legacy", "a", 21));
        let cache = crate::cache::Cache::open_at(&dir.join(".abcop-cache")).unwrap();
        let path = dir.join("a.rb");
        super::super::analyze_one(&path, None, limits(true), None, Some(&cache));
        let hit = super::super::analyze_one(&path, None, limits(true), None, Some(&cache));
        assert_eq!(hit.abc.len(), 1);
        assert!(hit.abc[0].prior.is_some());
        cleanup(&dir);
    }
}

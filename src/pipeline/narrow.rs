//! Changeset narrowing: a fresh or cached result is trimmed to the lines
//! this working-tree change actually touches, and ModuleSize follows the
//! refactor-scale rule.

use crate::git_changes;
use crate::output::FileResult;

/// Drops diagnostics outside the changed lines of this file.
pub(super) fn apply(
    changeset: Option<&git_changes::Changeset>,
    r: &mut FileResult,
    src: &[u8],
) {
    let Some(cs) = changeset else { return };
    let Some(rel) = cs.rel_of(&r.path) else {
        return;
    };
    r.abc.retain(|o| cs.span_selected(rel, o.line, o.end_line));
    r.used_once.retain(|o| cs.line_selected(rel, o.line));
    r.never_used.retain(|o| cs.line_selected(rel, o.line));
    let rel = rel.to_string();
    apply_module_size_policy(cs, &rel, src, r);
}

/// Scoped runs gate ModuleSize on refactor-scale diffs only -- for any
/// module, spec or production: a >=100-line diff invites the size
/// conversation even in tests, while small patches into legacy giants do
/// not. A missing oversize entry gains one when a test-tree diff is big
/// enough to make specs size-accountable.
fn apply_module_size_policy(
    cs: &git_changes::Changeset,
    rel: &str,
    src: &[u8],
    r: &mut FileResult,
) {
    let refactor_scale = changed_line_count(cs, rel) >= crate::modulesize::MIN_REVIEW_REFACTOR_LINES;
    if r.oversize.is_some() {
        if !refactor_scale {
            r.oversize = None;
        }
        return;
    }
    if !refactor_scale || !crate::modulesize::is_test_path(rel) {
        return;
    }
    let text = std::str::from_utf8(src).unwrap_or("");
    let lines = crate::modulesize::effective_lines(text, rel);
    r.oversize = (lines >= crate::modulesize::MAX_LINES).then_some(lines);
}

/// Touched-line count for a repo-relative path; untracked files count as
/// fully changed.
fn changed_line_count(cs: &git_changes::Changeset, rel: &str) -> usize {
    match cs.files.get(rel) {
        Some(git_changes::Lines::All) => usize::MAX,
        Some(git_changes::Lines::Ranges(set)) => set.len(),
        None => 0,
    }
}

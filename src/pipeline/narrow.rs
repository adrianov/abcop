//! Changeset narrowing: a fresh or cached result is trimmed to the lines
//! this working-tree change actually touches, and ModuleAbcSize follows the
//! refactor-scale rule.

use crate::git_changes;
use crate::modulesize;
use crate::output::FileResult;

/// Drops diagnostics outside the changed lines of this file, then applies
/// ModuleAbcSize's scoped-run policy.
pub(super) fn apply(changeset: Option<&git_changes::Changeset>, r: &mut FileResult, _src: &[u8]) {
    let Some(cs) = changeset else {
        modulesize::drop_non_production(&r.path, &mut r.module_abc);
        return;
    };
    let Some(rel) = cs.rel_of(&r.path) else {
        return;
    };
    let rel = rel.to_string();
    r.abc.retain(|o| cs.span_selected(&rel, o.line, o.end_line));
    r.used_once.retain(|o| cs.line_selected(&rel, o.line));
    r.never_used.retain(|o| cs.line_selected(&rel, o.line));
    apply_module_abc_policy(cs, &rel, r);
}

/// Scoped runs gate ModuleAbcSize on refactor-scale diffs only -- for any
/// module, spec or production: a >=100-line diff invites the size
/// conversation even in tests, while small patches into legacy giants do
/// not. Oversized test files keep their ModuleAbcSize hit once the diff
/// itself is refactor-scale (analysis already scored them).
fn apply_module_abc_policy(cs: &git_changes::Changeset, rel: &str, r: &mut FileResult) {
    let refactor_scale =
        changed_line_count(cs, rel) >= modulesize::MIN_REVIEW_REFACTOR_LINES;
    if r.module_abc.is_some() && !refactor_scale {
        r.module_abc = None;
    }
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

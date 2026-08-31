//! Changeset narrowing: a fresh or cached result is trimmed to the lines
//! this working-tree change actually touches, and size metrics follow the
//! refactor-scale [`crate::modulesize::SizeGate`] rule.

use crate::git_changes;
use crate::modulesize::{self, SizeGate};
use crate::output::FileResult;

/// Drops diagnostics outside the changed lines of this file, then applies
/// the scoped-run size gate for AbcSize and ModuleAbcSize.
pub(super) fn apply(
    changeset: Option<&git_changes::Changeset>,
    r: &mut FileResult,
    size_gate: SizeGate,
    _src: &[u8],
) {
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
    apply_size_gate(cs, &rel, r, size_gate);
}

/// Scoped runs can suppress AbcSize and ModuleAbcSize until the diff is
/// refactor-scale (≥100 touched lines). [`SizeGate`] selects which paths
/// that threshold covers: specs only, both production and specs, or none.
fn apply_size_gate(cs: &git_changes::Changeset, rel: &str, r: &mut FileResult, gate: SizeGate) {
    if !gate.covers(rel) {
        return;
    }
    let refactor_scale = changed_line_count(cs, rel) >= modulesize::MIN_REVIEW_REFACTOR_LINES;
    if refactor_scale {
        return;
    }
    r.module_abc = None;
    r.abc.clear();
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

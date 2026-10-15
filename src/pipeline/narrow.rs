//! Changeset narrowing: a fresh or cached result is trimmed to the lines
//! this working-tree change actually touches. ModuleAbcSize is a whole-file
//! score, so it is left for the HEAD edit filter.

use crate::git_changes;
use crate::modulesize;
use crate::output::FileResult;

/// Drops function and local diagnostics outside the changed lines.
/// A full scan (no changeset) drops ModuleAbcSize on non-production paths.
pub(super) fn apply(changeset: Option<&git_changes::Changeset>, r: &mut FileResult) {
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
}

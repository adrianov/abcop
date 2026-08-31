//! Changeset narrowing: a fresh or cached result is trimmed to the lines
//! this working-tree change actually touches, and ModuleAbcSize is
//! re-scored from changed methods only.

use crate::git_changes;
use crate::modulesize;
use crate::output::FileResult;

/// Drops diagnostics outside the changed lines of this file, then
/// re-scores ModuleAbcSize from methods that intersect the diff.
pub(super) fn apply(
    changeset: Option<&git_changes::Changeset>,
    r: &mut FileResult,
    max_module: f64,
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
    modulesize::rescope(&mut r.module_abc, max_module, |o| {
        cs.span_selected(&rel, o.line, o.end_line)
    });
}

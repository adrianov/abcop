//! C-family language backend: JavaScript, TypeScript, C, C++, Objective-C
//! and Swift AbcSize over the shared tree-sitter tree.
//!
//! Submodules: [`spec`] (per-language grammar tables), [`scan`] (unit
//! discovery and naming), [`tally`] (the ABC counter), [`tests`] (end-to-end
//! vector assertions per language).
//!
//! - A unit's score walks its whole body but never descends into another
//!   unit root -- those carry their own offense, so nothing double-counts.

mod scan;
mod spec;
mod tally;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::abc::{AbcOffense, fmt_vector};
use crate::paths::Lang;

use scan::{discover, unit_body};
use spec::{Spec, spec_for};
use tally::Tally;

pub(crate) fn analyze(src: &[u8], tree: &Tree, lang: Lang, max: f64) -> Vec<AbcOffense> {
    let spec = spec_for(lang);
    let mut units = Vec::new();
    let mut roots = HashSet::new();
    discover(&spec, tree.root_node(), src, &mut units, &mut roots);

    let mut offenses: Vec<AbcOffense> = units
        .into_iter()
        .filter_map(|(unit, name)| unit_offense(unit, name, &spec, src, &roots))
        .collect();
    offenses.sort_by_key(|o| (o.line, o.column));
    offenses.retain(|o| o.score > max);
    offenses
}

fn unit_offense(
    unit: Node,
    name: String,
    spec: &Spec,
    src: &[u8],
    roots: &HashSet<usize>,
) -> Option<AbcOffense> {
    let body = unit_body(unit)?;
    let mut t = Tally::new();
    t.walk(spec, body, src, roots);
    Some(score_offense(unit, name, t))
}

/// Vector and score for a finished tally, positioned at its unit root.
fn score_offense(unit: Node, name: String, t: Tally) -> AbcOffense {
    let (a, b, c) = t.counts();
    let pos = unit.start_position();
    let raw = ((a * a + b * b + c * c) as f64).sqrt();
    AbcOffense {
        line: pos.row + 1,
        end_line: unit.end_position().row + 1,
        column: pos.column,
        name,
        score: (raw * 100.0).round() / 100.0,
        vector: fmt_vector(a, b, c),
    }
}

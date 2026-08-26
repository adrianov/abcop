//! C-family language backend: AbcSize for JavaScript, TypeScript, C,
//! C++, Objective-C and Swift over the shared tree-sitter tree, plus
//! UsedOnce/NeverUsed for JavaScript, TypeScript and Swift.
//!
//! Submodules: [`spec`] (per-language grammar tables), [`scan`] (unit
//! discovery and naming), [`tally`] (the ABC counter), [`scope`] (the
//! JavaScript/TypeScript scope collector), [`swift`] (the Swift scope
//! collector), [`purity`] (shared RHS-purity predicates), [`tests`] and
//! [`tests_abc`] (end-to-end vector assertions).

mod scan;
mod scope;
mod spec;
mod tally;
mod purity;
mod c;
mod swift;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_swift;
#[cfg(test)]
mod tests_cfamily;
#[cfg(test)]
mod tests_abc;

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use crate::abc::{AbcOffense, fmt_vector};
use crate::never_used::NeverUsedOffense;
use crate::paths::Lang;
use crate::used_once::UsedOnceOffense;

/// Collected variable model for the JS/TS/Swift family.
pub(crate) struct JsScopes<'t> {
    pub scopes: Vec<crate::scope_model::Scope>,
    pub root: tree_sitter::Node<'t>,
}

pub(crate) fn collect_scopes<'t>(
    src: &[u8],
    tree: &'t Tree,
    lang: crate::paths::Lang,
) -> JsScopes<'t> {
    let scopes = match lang {
        Lang::Js | Lang::Ts | Lang::Tsx => scope::collect(tree.root_node(), src),
        Lang::Swift => swift::swift_collect(tree.root_node(), src),
        Lang::C | Lang::Cpp | Lang::ObjC => c::collect(tree.root_node(), src, lang),
        other => unreachable!("clike scope backend for lang: {other:?}"),
    };
    JsScopes { scopes, root: tree.root_node() }
}

static JS_SEMANTICS: crate::scope_model::Semantics = crate::scope_model::Semantics {
    pure: purity::js_pure,
    veto: &[
        "if_statement",
        "for_statement",
        "for_in_statement",
        "for_of_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "try_statement",
        "catch_clause",
    ],
    owners: &["function_declaration", "generator_function_declaration", "method_definition"],
    include_root_scope: false,
};

static SWIFT_SEMANTICS: crate::scope_model::Semantics = crate::scope_model::Semantics {
    pure: purity::swift_pure,
    veto: &[
        "if_statement",
        "guard_statement",
        "for_statement",
        "while_statement",
        "repeat_while_statement",
        "switch_statement",
        "do_catch_statement",
    ],
    owners: &["function_declaration", "init_declaration", "closure_expression"],
    include_root_scope: false,
};

fn semantics_for(lang: crate::paths::Lang) -> &'static crate::scope_model::Semantics {
    match lang {
        Lang::Js | Lang::Ts | Lang::Tsx => &JS_SEMANTICS,
        Lang::Swift => &SWIFT_SEMANTICS,
        Lang::C | Lang::Cpp | Lang::ObjC => &C_FAMILY_SEMANTICS,
        other => unreachable!("no semantics for non-clike lang: {other:?}"),
    }
}

static C_FAMILY_SEMANTICS: crate::scope_model::Semantics = crate::scope_model::Semantics {
    pure: purity::c_like_pure,
    veto: &[
        "if_statement",
        "for_statement",
        "for_range_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "case_statement",
        "try_statement",
        "catch_clause",
    ],
    owners: &["function_definition", "method_definition"],
    include_root_scope: false,
};

pub(crate) fn used_once_offenses(
    sc: &JsScopes,
    lang: crate::paths::Lang,
) -> Vec<UsedOnceOffense> {
    crate::scope_model::used_once_offenses(sc.root, &|b| line_col(sc.root, b), &sc.scopes, semantics_for(lang))
 }
 
pub(crate) fn never_used_offenses(
    sc: &JsScopes,
    lang: crate::paths::Lang,
) -> Vec<NeverUsedOffense> {
    crate::scope_model::never_used_offenses(&|b| line_col(sc.root, b), &sc.scopes, semantics_for(lang))
 }

fn line_col(root: tree_sitter::Node, byte: usize) -> (usize, usize) {
    let point = root
        .descendant_for_byte_range(byte, byte)
        .map(|n| n.start_position())
        .unwrap_or_default();
    (point.row + 1, point.column)
}

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

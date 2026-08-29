//! Language-backend dispatch: routes a parsed file to its engine and
//! applies inline suppression directives on top of its diagnostics.

use tree_sitter::Tree;

use crate::abc::{AbcOffense, Limits};
use crate::directives::{self, Directives};
use crate::modulesize;
use crate::never_used::NeverUsedOffense;
use crate::output::FileResult;
use crate::used_once::UsedOnceOffense;

use super::Checks;
use super::non_clike::{CSharpB, DartB, GoB, JavaB, NonClike, PhpB, PyB, SolidityB};
use crate::paths::Lang;

/// Inline directives, parsed once per file.
fn directives_for(src: &[u8]) -> Directives {
    directives::parse(&String::from_utf8_lossy(src))
}

/// Drops entries the inline directives suppress.
fn suppressed<T>(v: Vec<T>, keep: impl Fn(&T) -> bool) -> Vec<T> {
    v.into_iter().filter(|o| !keep(o)).collect()
}

fn set_module_abc(r: &mut FileResult, src: &[u8], all: &[AbcOffense], max: f64) {
    let text = std::str::from_utf8(src).unwrap_or("");
    r.module_abc = modulesize::from_scores(all, &r.path, text, max);
}

fn keep_abc(dirs: &Directives, all: Vec<AbcOffense>, max: f64) -> Vec<AbcOffense> {
    suppressed(
        all.into_iter().filter(|o| o.score > max).collect(),
        |o| dirs.suppresses_abc(o.line),
    )
}

/// Run the clike scope-model family (JS/TS, Swift and the plain-C trio):
/// ABC plus UsedOnce/NeverUsed for grammars with collectors.
pub(super) fn clike_arm(
    r: &mut FileResult,
    lang: Lang,
    src: &[u8],
    tree: &Tree,
    checks: &Checks,
    limits: Limits,
) {
    let dirs = directives_for(src);
    let all = crate::clike::all_scores(src, tree, lang);
    set_module_abc(r, src, &all, limits.module);
    if checks.want_abc {
        r.abc = keep_abc(&dirs, all, limits.method);
    }
    if !matches!(
        lang,
        Lang::Js | Lang::Ts | Lang::Tsx | Lang::Swift | Lang::C | Lang::Cpp | Lang::ObjC
    ) {
        return;
    }
    let scopes = crate::clike::collect_scopes(src, tree, lang);
    if checks.want_used {
        r.used_once = suppressed(crate::clike::used_once_offenses(&scopes, lang), |o| {
            dirs.suppresses_all(o.line)
        });
    }
    if checks.want_never {
        r.never_used = crate::clike::never_used_offenses(&scopes, lang);
    }
}

/// Shared driver for directive-aware non-clike backends: one build site,
/// three optionally-run checks with inline filtering applied to abc/used.
fn directed<A, U, N>(
    r: &mut FileResult,
    src: &[u8],
    dirs: &Directives,
    checks: &Checks,
    limits: Limits,
    all_scores: A,
    used_once: U,
    never_used: N,
) where
    A: FnOnce() -> Vec<AbcOffense>,
    U: FnOnce() -> Vec<UsedOnceOffense>,
    N: FnOnce() -> Vec<NeverUsedOffense>,
{
    let all = all_scores();
    set_module_abc(r, src, &all, limits.module);
    if checks.want_abc {
        r.abc = keep_abc(dirs, all, limits.method);
    }
    if checks.want_used {
        r.used_once = suppressed(used_once(), |o| dirs.suppresses_all(o.line));
    }
    if checks.want_never {
        r.never_used = never_used();
    }
}

/// Directive-aware non-clike backends. Always succeeds; the bool keeps
/// parity with the Ruby arm's signature.
fn non_clike_directed<B: NonClike>(
    r: &mut FileResult,
    src: &[u8],
    tree: Tree,
    checks: &Checks,
    limits: Limits,
) -> bool {
    let fm = B::build(src, tree);
    directed(
        r,
        src,
        &directives_for(src),
        checks,
        limits,
        || B::all_scores(&fm),
        || B::used_once_offenses(&fm),
        || B::never_used_offenses(&fm),
    );
    true
}

/// Rust backend: no inline directives (rustc/clippy own that noise); all
/// three rules run.
fn rust_arm(
    r: &mut FileResult,
    src: &[u8],
    tree: Tree,
    checks: &Checks,
    limits: Limits,
) -> bool {
    let fm = crate::rustlang::build(src, tree);
    let all = crate::rustlang::all_scores(&fm);
    set_module_abc(r, src, &all, limits.module);
    if checks.want_abc {
        r.abc = all
            .into_iter()
            .filter(|o| o.score > limits.method)
            .collect();
    }
    if checks.want_used {
        r.used_once = crate::rustlang::used_once_offenses(&fm);
    }
    if checks.want_never {
        r.never_used = crate::rustlang::never_used_offenses(&fm);
    }
    true
}

/// Ruby backend: directives-filtered ABC and used-once, plain never-used.
/// False when the Ruby reparse fails (no usable model).
fn ruby_arm(r: &mut FileResult, src: &[u8], checks: &Checks, limits: Limits) -> bool {
    let dirs = directives_for(src);
    let Some(fm) = super::reparsed(src, Lang::Ruby) else {
        return false;
    };
    let all = crate::abc::all_scores(&fm);
    set_module_abc(r, src, &all, limits.module);
    if checks.want_abc {
        r.abc = keep_abc(&dirs, all, limits.method);
    }
    if checks.want_used {
        r.used_once = suppressed(crate::used_once::analyze(&fm), |o| {
            dirs.suppresses_all(o.line)
        });
    }
    if checks.want_never {
        r.never_used = crate::never_used::analyze(&fm);
    }
    true
}

/// Route a non-clike language to its backend. False when the Ruby reparse
/// fails (no usable model).
pub(super) fn non_clike_arm(
    r: &mut FileResult,
    lang: Lang,
    src: &[u8],
    tree: Tree,
    checks: &Checks,
    limits: Limits,
) -> bool {
    match lang {
        Lang::Rust => rust_arm(r, src, tree, checks, limits),
        Lang::Ruby => ruby_arm(r, src, checks, limits),
        Lang::Py => non_clike_directed::<PyB>(r, src, tree, checks, limits),
        Lang::Go => non_clike_directed::<GoB>(r, src, tree, checks, limits),
        Lang::Php => non_clike_directed::<PhpB>(r, src, tree, checks, limits),
        Lang::Java => non_clike_directed::<JavaB>(r, src, tree, checks, limits),
        Lang::CSharp => non_clike_directed::<CSharpB>(r, src, tree, checks, limits),
        Lang::Solidity => non_clike_directed::<SolidityB>(r, src, tree, checks, limits),
        Lang::Dart => non_clike_directed::<DartB>(r, src, tree, checks, limits),
        _ => unreachable!("unsupported non-clike language"),
    }
}

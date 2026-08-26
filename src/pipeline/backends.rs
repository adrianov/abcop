//! Language-backend dispatch: routes a parsed file to its engine and
//! applies inline suppression directives on top of its diagnostics.

use tree_sitter::Tree;

use crate::abc::AbcOffense;
use crate::directives::{self, Directives};
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

/// Run the clike scope-model family (JS/TS, Swift and the plain-C trio):
/// ABC plus UsedOnce/NeverUsed for grammars with collectors.
pub(super) fn clike_arm(
    r: &mut FileResult,
    lang: Lang,
    src: &[u8],
    tree: &Tree,
    checks: &Checks,
    max: f64,
) {
    let dirs = directives_for(src);
    if checks.want_abc {
        r.abc = suppressed(crate::clike::analyze(src, tree, lang, max), |o| {
            dirs.suppresses_abc(o.line)
        });
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
    dirs: &Directives,
    checks: &Checks,
    max: f64,
    analyze_abc: A,
    used_once: U,
    never_used: N,
) where
    A: FnOnce(f64) -> Vec<AbcOffense>,
    U: FnOnce() -> Vec<UsedOnceOffense>,
    N: FnOnce() -> Vec<NeverUsedOffense>,
{
    if checks.want_abc {
        r.abc = suppressed(analyze_abc(max), |o| dirs.suppresses_abc(o.line));
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
    max: f64,
) -> bool {
    let fm = B::build(src, tree);
    directed(
        r,
        &directives_for(src),
        checks,
        max,
        |max| B::analyze(&fm, max),
        || B::used_once_offenses(&fm),
        || B::never_used_offenses(&fm),
    );
    true
}

/// Rust backend: no inline directives (rustc/clippy own that noise); all
/// three rules run.
fn rust_arm(r: &mut FileResult, src: &[u8], tree: Tree, checks: &Checks, max: f64) -> bool {
    let fm = crate::rustlang::build(src, tree);
    if checks.want_abc {
        r.abc = crate::rustlang::analyze(&fm, max);
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
fn ruby_arm(r: &mut FileResult, src: &[u8], checks: &Checks, max: f64) -> bool {
    let dirs = directives_for(src);
    let Some(fm) = super::reparsed(src, Lang::Ruby) else {
        return false;
    };
    if checks.want_abc {
        r.abc = suppressed(crate::abc::analyze(&fm, max), |o| {
            dirs.suppresses_abc(o.line)
        });
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
    max: f64,
) -> bool {
    match lang {
        Lang::Rust => rust_arm(r, src, tree, checks, max),
        Lang::Ruby => ruby_arm(r, src, checks, max),
        Lang::Py => non_clike_directed::<PyB>(r, src, tree, checks, max),
        Lang::Go => non_clike_directed::<GoB>(r, src, tree, checks, max),
        Lang::Php => non_clike_directed::<PhpB>(r, src, tree, checks, max),
        Lang::Java => non_clike_directed::<JavaB>(r, src, tree, checks, max),
        Lang::CSharp => non_clike_directed::<CSharpB>(r, src, tree, checks, max),
        Lang::Solidity => non_clike_directed::<SolidityB>(r, src, tree, checks, max),
        Lang::Dart => non_clike_directed::<DartB>(r, src, tree, checks, max),
        _ => unreachable!("unsupported non-clike language"),
    }
}

//! UsedOnce candidacy and NeverUsed reporting over the Python backend.

use super::{build, never_used_offenses, used_once_offenses};
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::pylang::PyFile<'static> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Py).expect("python parses");
    build(src.as_bytes(), tree)
}

fn used(src: &'static str) -> Vec<String> {
    let mut v: Vec<_> = used_once_offenses(&parse(src))
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn dead(src: &'static str) -> Vec<String> {
    let mut v: Vec<_> = never_used_offenses(&parse(src))
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

#[test]
fn attribute_and_subscript_targets_bind_nothing() {
    let got = dead(
        "def targets(o, k):\n\
                    \x20   o.a = 1\n\
                    \x20   o[k] = 2\n\
                    \x20   return o\n",
    );
    assert!(
        got.is_empty(),
        "reference targets are not bindings: {got:?}"
    );
}

#[test]
fn used_once_flags_single_pure_straightline_binding() {
    assert_eq!(
        used("def ok(f):\n    x = 42\n    return x + f\n"),
        vec!["x"]
    );
}

#[test]
fn used_once_rejections() {
    let src = "def rej(f):\n\
               \x20   a = f\n\
               \x20   b = 1\n\
               \x20   b = 2\n\
               \x20   c = 1\n\
               \x20   c += 1\n\
               \x20   d = id(a)\n\
               \x20   if f:\n\
               \x20       e = 1\n\
               \x20   _u = 3\n\
               \x20   return d\n";
    assert_eq!(used(src), Vec::<String>::new());
}

#[test]
fn never_used_reports_dead_writes_once() {
    let src = "def dd(p):\n\
               \x20   unused = 1\n\
               \x20   q, r = 2, 3\n\
               \x20   return r\n";
    assert_eq!(dead(src), vec!["q", "unused"]);
}

#[test]
fn never_used_ignores_loop_targets_and_underscore() {
    let src = "def proto(path):\n\
               \x20   for i in range(3):\n\
               \x20       pass\n\
               \x20   _ignored = 9\n";
    assert_eq!(dead(src), Vec::<String>::new());
}

#[test]
fn never_used_flags_unread_with_and_except_aliases() {
    // Mirrors the Ruby backend, where an unread rescue variable is
    // flagged: `with ... as` / `except ... as` names are real bindings.
    let src = "def proto(path):\n\
               \x20   with open(path) as fh:\n\
               \x20       pass\n\
               \x20   try:\n\
               \x20       pass\n\
               \x20   except ValueError as err:\n\
               \x20       pass\n";
    assert_eq!(dead(src), vec!["err", "fh"]);
}

//! UsedOnce / NeverUsed contract vectors for the Go backend.

use super::{build, never_used_offenses, used_once_offenses};
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::golang::GoFile<'static> {
    build(
        src.as_bytes(),
        parse_file_lang(src.as_bytes(), Lang::Go).expect("go parses"),
    )
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
fn used_once_flags_pure_literal() {
    assert_eq!(
        used("package m\nfunc ok(f int) int {\n  x := 42\n  return x + f\n}\n"),
        vec!["x"]
    );
}

#[test]
fn used_once_immediate_call_chain_yes_intervening_and_loop_no() {
    assert_eq!(
        used("package m\nfunc ok() int {\n  a := id()\n  return a\n}\n"),
        vec!["a"]
    );
    assert_eq!(
        used("package m\nfunc no() int {\n  a := id()\n  side()\n  return a\n}\n"),
        Vec::<String>::new()
    );
    assert_eq!(
        used(
            "package m\nfunc loop(items []int) {\n  a := id()\n  for _, i := range items {\n    use(a, i)\n  }\n}\n"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn used_once_bare_alias_and_multi_assign_no() {
    assert_eq!(
        used("package m\nfunc ok(items int) int {\n  a := items\n  return a\n}\n"),
        vec!["a"]
    );
    assert_eq!(
        used("package m\nfunc pair(xs []int) int {\n  a, b := xs[0], xs[1]\n  return a + b\n}\n"),
        Vec::<String>::new()
    );
}

#[test]
fn never_used_dead_call_keeps_initializer() {
    let f = never_used_offenses(&parse(
        "package m\nfunc dd() int {\n  gone := id()\n  return 0\n}\n",
    ));
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "gone");
    assert!(f[0].keep_init);
}

#[test]
fn never_used_reports_unread_local() {
    assert_eq!(
        dead("package m\nfunc dd() int {\n  unused := 1\n  return 0\n}\n"),
        vec!["unused"]
    );
}

//! End-to-end assertions over the Python backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::pylang::PyFile<'static> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Py).expect("python parses");
    build(src.as_bytes(), tree)
}

fn scores(src: &'static str) -> Vec<(String, u32, u32, u32)> {
    let fm = parse(src);
    super::abc::all_scores(&fm)
        .into_iter()
        .map(|o| {
            let v = o.vector;
            let nums = v.trim_matches(|c| c == '<' || c == '>');
            let mut it = nums.split(", ");
            (
                o.name,
                it.next().unwrap().parse().unwrap(),
                it.next().unwrap().parse().unwrap(),
                it.next().unwrap().parse().unwrap(),
            )
        })
        .collect()
}

fn used(src: &'static str) -> Vec<String> {
    let fm = parse(src);
    let mut v: Vec<_> = used_once_offenses(&fm)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn dead(src: &'static str) -> Vec<String> {
    let fm = parse(src);
    let mut v: Vec<_> = never_used_offenses(&fm)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

#[test]
fn assignment_and_binary_count_as_a_and_b() {
    assert_eq!(
        scores("def simple(a):\n    x = a + 1\n    return x\n"),
        vec![("simple".to_string(), 1, 1, 0)]
    );
}

#[test]
fn branches_loops_and_conditions_tally_c() {
    // A: total, i target, +=, -= ; B: unary minus of -5
    // C: for, if, cmp, cmp, and, elif, cmp
    let src = "def branchy(items):\n\
               \x20   total = 0\n\
               \x20   for i in items:\n\
               \x20       if i > 0 and i != 5:\n\
               \x20           total += i\n\
               \x20       elif i < -5:\n\
               \x20           total -= 1\n\
               \x20   return total\n";
    assert_eq!(scores(src), vec![("branchy".to_string(), 4, 1, 7)]);
}

#[test]
fn comprehensions_add_loop_target_and_clauses() {
    // A: squares target, x comprehension target ; B: ** ; C: for_in, if_clause, cmp
    let src = "def comp(items):\n\
               \x20   squares = [x ** 2 for x in items if x > 0]\n\
               \x20   return squares\n";
    assert_eq!(scores(src), vec![("comp".to_string(), 2, 1, 3)]);
}

#[test]
fn try_except_match_ternary_count() {
    // A: r x2, t ; C: except, case x2, ternary
    let src = "def messy(p):\n\
               \x20   try:\n\
               \x20       r = 1\n\
               \x20   except ValueError:\n\
               \x20       r = 2\n\
               \x20   match p:\n\
               \x20       case 0:\n\
               \x20           pass\n\
               \x20       case other:\n\
               \x20           pass\n\
               \x20   t = 1 if p else 2\n\
               \x20   return (r, t)\n";
    assert_eq!(scores(src), vec![("messy".to_string(), 3, 0, 4)]);
}

#[test]
fn nested_functions_are_separate_units() {
    let src = "def outer():\n\
               \x20   def inner(k):\n\
               \x20       return k * 2\n\
               \x20   return inner(3)\n";
    let got = scores(src);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], ("outer".to_string(), 0, 1, 0));
    assert_eq!(got[1], ("inner".to_string(), 0, 1, 0));
}

#[test]
fn lambda_rolls_into_enclosing_unit() {
    let src = "def roller(fs):\n\
               \x20   g = lambda v: v + 1\n\
               \x20   return g(fs)\n";
    // A: g ; B: + inside lambda body, call g(fs)
    assert_eq!(scores(src), vec![("roller".to_string(), 1, 2, 0)]);
}

#[test]
fn methods_inside_classes_are_units() {
    let src = "class K:\n\
               \x20   def m(self):\n\
               \x20       self.v = 1\n\
               \x20       return self.v\n";
    // A: 0 (attribute target binds nothing)
    // B: two attribute reads -- target operand and return operand
    assert_eq!(scores(src), vec![("m".to_string(), 0, 2, 0)]);
}

#[test]
fn attribute_and_subscript_targets_bind_nothing() {
    let src = "def targets(o, k):\n\
               \x20   o.a = 1\n\
               \x20   o[k] = 2\n\
               \x20   return o\n";
    let got = dead(src);
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

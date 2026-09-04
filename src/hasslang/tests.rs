//! End-to-end assertions over the Haskell backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::abc::parse_vector;
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::hasslang::HsFile<'static> {
    build(
        src.as_bytes(),
        parse_file_lang(src.as_bytes(), Lang::Hs).expect("haskell parses"),
    )
}

fn scores(src: &'static str) -> Vec<(String, u32, u32, u32)> {
    super::abc::all_scores(&parse(src))
        .into_iter()
        .map(|o| {
            let (a, b, c) = parse_vector(&o.vector);
            (o.name, a, b, c)
        })
        .collect()
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

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/fixtures/haskell/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(path).expect("fixture")
}

fn scores_owned(src: &str) -> Vec<(String, u32, u32, u32)> {
    super::abc::all_scores(&build(
        src.as_bytes(),
        parse_file_lang(src.as_bytes(), Lang::Hs).expect("haskell parses"),
    ))
    .into_iter()
    .map(|o| {
        let (a, b, c) = parse_vector(&o.vector);
        (o.name, a, b, c)
    })
    .collect()
}

#[test]
fn abc_top_level_function_vector() {
    // A: none (params protocol); B: + ; C: none
    assert_eq!(
        scores("add a b = a + b\n"),
        vec![("add".into(), 0, 1, 0)]
    );
}

#[test]
fn abc_value_bind_is_a_unit() {
    assert_eq!(scores("answer = 42\n"), vec![("answer".into(), 0, 0, 0)]);
}

#[test]
fn abc_let_bind_counts_as_assignment() {
    // A: y; B: + and *; C: none
    assert_eq!(
        scores("f x = let y = x + 1 in y * 2\n"),
        vec![("f".into(), 1, 2, 0)]
    );
}

#[test]
fn abc_where_function_is_separate_unit() {
    let got = scores("f x = bar x\n  where\n    bar y = y + 1\n");
    assert_eq!(
        got,
        vec![("f".into(), 0, 1, 0), ("bar".into(), 0, 1, 0)]
    );
}

#[test]
fn abc_case_alternatives_and_pattern_binds() {
    // A: n; B: +; C: two alternatives
    assert_eq!(
        scores("f x = case x of\n  Just n -> n + 1\n  Nothing -> 0\n"),
        vec![("f".into(), 1, 1, 2)]
    );
}

#[test]
fn abc_conditional_comparisons_and_call() {
    // B: abs apply; C: conditional, >, &&, <
    assert_eq!(
        scores("f x = if x > 0 && x < 10 then abs x else 0\n"),
        vec![("f".into(), 0, 1, 4)]
    );
}

#[test]
fn abc_do_bind_let_and_call() {
    // A: x (<-), y (let); B: print; C: none
    assert_eq!(
        scores("f = do\n  x <- getLine\n  let y = x\n  print y\n"),
        vec![("f".into(), 2, 1, 0)]
    );
}

#[test]
fn abc_lambda_rolls_into_enclosing_unit() {
    // A: lambda x; B: map apply, outer apply, +; C: none
    assert_eq!(
        scores("f xs = map (\\x -> x + 1) xs\n"),
        vec![("f".into(), 1, 3, 0)]
    );
}

#[test]
fn abc_guards_count_conditions() {
    // C: boolean, >, boolean
    assert_eq!(
        scores("f x\n  | x > 0 = x\n  | otherwise = 0\n"),
        vec![("f".into(), 0, 0, 3)]
    );
}

#[test]
fn abc_list_comprehension() {
    // A: x generator, y let; B: +; C: boolean, >
    assert_eq!(
        scores("f xs = [y | x <- xs, x > 0, let y = x + 1]\n"),
        vec![("f".into(), 2, 1, 2)]
    );
}

#[test]
fn abc_fixture_branchy() {
    let got = scores_owned(&fixture("branchy.hs"));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "branchy");
    // A: n (case), total (let), v (<-); B: +, pure, print, pure; C: alts×2, conditional, ==
    assert_eq!(got[0].1, 3, "assignments/binders: {:?}", got);
    assert_eq!(got[0].2, 4, "calls/ops: {:?}", got);
    assert_eq!(got[0].3, 4, "conditions: {:?}", got);
}

#[test]
fn abc_fixture_nested_where() {
    let mut got = scores_owned(&fixture("nested_where.hs"));
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        got,
        vec![("helper".into(), 0, 1, 0), ("outer".into(), 0, 1, 0)]
    );
}

#[test]
fn used_once_inline_candidate_for_pure_literal() {
    assert_eq!(
        used("f x = let dead = 5 in dead\n"),
        vec!["dead"]
    );
}

#[test]
fn used_once_immediate_call_yes_intervening_no() {
    assert_eq!(
        used("f b = let g = compute b in g\n"),
        vec!["g"]
    );
    assert_eq!(
        used("f b = let g = compute b in side () `seq` g\n"),
        Vec::<String>::new()
    );
}

#[test]
fn never_used_dead_call_keeps_initializer() {
    let f = never_used_offenses(&parse(
        "f = let gone = compute 1 in 0\n",
    ));
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "gone");
    assert!(f[0].keep_init);
}

#[test]
fn used_once_params_and_patterns_are_protocol() {
    assert_eq!(
        used("f p = case p of\n  Just v -> v\n  Nothing -> 0\n"),
        Vec::<String>::new()
    );
}

#[test]
fn used_once_conditional_let_is_vetoed() {
    assert_eq!(
        used("f = if True then let y = 1 in y else 0\n"),
        Vec::<String>::new()
    );
}

#[test]
fn used_once_let_inside_lambda_is_candidate() {
    assert_eq!(
        used("f xs = map (\\x -> let y = 5 in y) xs\n"),
        vec!["y"]
    );
}

#[test]
fn never_used_reports_locals_but_not_module_binds() {
    assert_eq!(
        dead("module M where\nG = 1\nm q = let lost = 1 in q\n"),
        vec!["lost"]
    );
}

#[test]
fn never_used_unused_parameter_is_exempt() {
    // Parameters are protocol Binding; NeverUsed leaves them alone.
    assert_eq!(dead("m q = 0\n"), Vec::<String>::new());
}

#[test]
fn never_used_class_method_parameter_is_exempt() {
    assert_eq!(
        dead("class C a where\n  op :: a -> a\n  op x = 0\n"),
        Vec::<String>::new()
    );
}

#[test]
fn abc_class_instance_methods_are_units() {
    let got = scores(
        "class C a where\n  op :: a -> a\n  op x = x\ninstance C Int where\n  op n = n + 1\n",
    );
    assert_eq!(
        got,
        vec![("op".into(), 0, 0, 0), ("op".into(), 0, 1, 0)]
    );
}

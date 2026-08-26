//! End-to-end assertions over the C# backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::paths::{parse_file_lang, Lang};

fn parse(src: &'static str) -> crate::csharp::CSharpFile<'static> {
    let tree = parse_file_lang(src.as_bytes(), Lang::CSharp).expect("csharp parses");
    build(src.as_bytes(), tree)
}

fn scores(src: &'static str) -> Vec<(String, u32, u32, u32)> {
    let fm = parse(src);
    super::abc::all_scores(&fm)
        .into_iter()
        .map(|o| {
            let nums = o.vector.trim_matches(|c| c == '<' || c == '>');
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
        scores("class K {\n  int Simple(int a) {\n    var x = a + 1;\n    return x;\n  }\n}"),
        vec![("Simple".to_string(), 1, 1, 0)]
    );
}

#[test]
fn branches_loops_sections_tally_c() {
    // A: total declarator, foreach head it, += = 3 (no invocations here)
    // B: none -- member reads are free
    // C: foreach, if, ==, ||, >, elseif-if, <, switch section x2 = 9
    let src = "class K {\n  int Run(System.Collections.Generic.List<string> items, int limit) {\n\
               \x20   var total = 0;\n\
               \x20   foreach (var it in items) {\n\
               \x20     if (it == null || it.Length > limit) { total += it.Length; }\n\
               \x20     else if (total < 0) { break; }\n\
               \x20   }\n\
               \x20   switch (total) {\n\
               \x20     case 0: break;\n\
               \x20     default: break;\n\
               \x20   }\n\
               \x20   return total;\n\
               \x20 }\n}";
    assert_eq!(scores(src), vec![("Run".to_string(), 3, 0, 9)]);
}

#[test]
fn constructors_are_units_and_lambdas_roll_in() {
    let src = "class K {\n  int seed;\n  K(int seed) {\n\
               \x20   this.seed = seed;\n\
               \x20   System.Func<int,int> s = v => v * 2;\n\
               \x20   Use(s);\n\
               \x20 }\n}";
    let got = scores(src);
    assert_eq!(got.len(), 1);
    // A: the local `s` declarator (`this.seed` is a field target)
    // B: * inside the lambda and the Use(...) invocation
    assert_eq!(got[0], ("K".to_string(), 1, 2, 0));
}

#[test]
fn member_names_are_not_variable_reads() {
    let src = "class K {\n  int count;\n  void M(K other) {\n\
               \x20   other.count = 1;\n\
               \x20   System.Console.WriteLine(other.count);\n\
               \x20 }\n}";
    assert_eq!(
        dead(src),
        Vec::<String>::new(),
        "field writes are not local bindings"
    );
}

#[test]
fn used_once_flags_single_pure_straightline_binding() {
    assert_eq!(
        used("class K {\n  int Ok(int f) {\n    var x = 42;\n    return x + f;\n  }\n}"),
        vec!["x"]
    );
}

#[test]
fn used_once_rejections() {
    let src = "class K {\n  int Rej(bool f) {\n\
               \x20   var a = Id();\n\
               \x20   var b = 1; b = 2;\n\
               \x20   var c = 1; c += 1;\n\
               \x20   if (f) { var e = 1; }\n\
               \x20   return a;\n\
               \x20 }\n}";
    assert_eq!(used(src), Vec::<String>::new());
}

#[test]
fn never_used_reports_dead_writes_once() {
    let src = "class K {\n  int Dd() {\n    var unused = 1;\n    return 0;\n  }\n}";
    assert_eq!(dead(src), vec!["unused"]);
}

#[test]
fn protocol_bindings_are_exempt() {
    let src = "class K {\n  void Proto(System.Collections.Generic.List<int> items) {\n\
               \x20   foreach (var i in items) { System.Console.WriteLine(i); }\n\
               \x20   try { throw new System.Exception(\"x\"); }\n\
               \x20   catch (System.Exception e) { }\n\
               \x20 }\n}";
    assert_eq!(dead(src), vec!["e"]);
}

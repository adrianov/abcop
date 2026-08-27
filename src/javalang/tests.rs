//! End-to-end assertions over the Java backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::abc::parse_vector;
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::javalang::JavaFile<'static> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Java).expect("java parses");
    build(src.as_bytes(), tree)
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
        scores("class K {\n  int simple(int a) {\n    int x = a + 1;\n    return x;\n  }\n}"),
        vec![("simple".to_string(), 1, 1, 0)]
    );
}

#[test]
fn branches_loops_switch_labels_tally_c() {
    // A: total, it(head), += ; B: two length() invocations
    // C: enhanced-for, if, ==, ||, >, elseif-if, <, switch label x2
    let src = "class K {\n  int run(java.util.List<String> items, int limit) {\n\
               \x20   int total = 0;\n\
               \x20   for (String it : items) {\n\
               \x20     if (it == null || it.length() > limit) continue;\n\
               \x20     else if (total < 0) break;\n\
               \x20     total += it.length();\n\
               \x20   }\n\
               \x20   switch (total) { case 0 -> total++; default -> total--; }\n\
               \x20   return total;\n\
               \x20 }\n}";
    assert_eq!(scores(src), vec![("run".to_string(), 5, 2, 9)]);
}

#[test]
fn constructors_are_units_and_lambdas_roll_in() {
    let got = scores(
        "class K {\n  int seed;\n  K(int seed) {\n\
                     \x20   this.seed = seed;\n\
                     \x20   java.util.function.IntSupplier s = () -> 40 + 2;\n\
                     \x20   useLater(s);\n\
                     \x20 }\n}",
    );
    assert_eq!(got.len(), 1);
    // A: the local `s` declarator (field target binds nothing)
    // B: + inside the lambda and the useLater(...) invocation
    assert_eq!(got[0], ("K".to_string(), 1, 2, 0));
}

#[test]
fn member_names_are_not_variable_reads() {
    let src =
        "class K {\n  int m(K other) {\n    other.count = 1;\n    return other.count;\n  }\n}";
    // A: 0 (field targets bind nothing); B: none beyond assignments? the
    // two field accesses are reads of `other` only; assignments to fields
    // contribute no A. C: 0.
    assert_eq!(
        dead(src),
        Vec::<String>::new(),
        "field writes are not local bindings"
    );
}

#[test]
fn used_once_flags_single_pure_straightline_binding() {
    assert_eq!(
        used("class K {\n  int ok(int f) {\n    int x = 42;\n    return x + f;\n  }\n}"),
        vec!["x"]
    );
}

#[test]
fn used_once_rejections() {
    let src = "class K {\n  int rej(boolean f) {\n\
               \x20   int a = id();\n\
               \x20   int b = 1; b = 2;\n\
               \x20   int c = 1; c += 1;\n\
               \x20   if (f) { int e = 1; }\n\
               \x20   return a;\n\
               \x20 }\n}";
    assert_eq!(used(src), Vec::<String>::new());
}

#[test]
fn never_used_reports_dead_writes_once() {
    let src = "class K {\n  int dd() {\n    int unused = 1;\n    return 0;\n  }\n}";
    assert_eq!(dead(src), vec!["unused"]);
}

#[test]
fn protocol_bindings_are_exempt() {
    let src = "class K {\n  void proto(java.util.List<Integer> items) {\n\
               \x20   for (int i = 0; i < items.size(); i++) { System.out.println(items); }\n\
               \x20   try (var res = open()) { res.toString(); }\n\
               \x20   catch (RuntimeException ex) { }\n\
               \x20 }\n}";
    assert_eq!(dead(src), vec!["ex"]);
}

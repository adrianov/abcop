//! End-to-end assertions over the Zig backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::abc::parse_vector;
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::ziglang::ZigFile<'static> {
    build(
        src.as_bytes(),
        parse_file_lang(src.as_bytes(), Lang::Zig).expect("zig parses"),
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

#[test]
fn abc_top_level_function_vector() {
    // A: `x` declarator; B: additive; C: none
    assert_eq!(
        scores("fn add(a: i32, b: i32) i32 {\n  const x = a + b;\n  return x;\n}"),
        vec![("add".into(), 1, 1, 0)]
    );
}

#[test]
fn abc_branches_calls_and_payloads() {
    // A: total, |v|, |item|, += ; B: print; C: if, for, if, ==, catch
    let got = scores(
        r#"fn branchy(flag: bool, opt: ?i32) void {
  var total: i32 = 0;
  if (opt) |v| {
    total += v;
  }
  for ([_]i32{1, 2}) |item| {
    _ = item;
  }
  if (flag == true) {
    print();
  }
  _ = error.Fail catch {};
}"#,
    );
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "branchy");
    assert_eq!(got[0].1, 4, "assignments/payloads");
    assert_eq!(got[0].2, 1, "calls");
    assert_eq!(got[0].3, 5, "branches/conditions");
}

#[test]
fn abc_comptime_block_is_a_unit() {
    assert_eq!(
        scores("comptime {\n  const x = 1 + 2;\n  _ = x;\n}"),
        vec![("comptime".into(), 1, 1, 0)]
    );
}

#[test]
fn abc_nested_method_not_rolled_into_outer() {
    let got = scores(
        r#"fn outer() void {
  const S = struct {
    fn inner() void {
      const dead = 1;
      _ = dead;
    }
  };
  _ = S;
}"#,
    );
    assert_eq!(
        got,
        vec![("outer".into(), 1, 0, 0), ("inner".into(), 1, 0, 0)]
    );
}

#[test]
fn abc_while_continue_counts_assign_and_conditions() {
    // A: i, v, += ; C: while, <, if, ==
    assert_eq!(
        scores(
            r#"fn f() i32 {
  var i: i32 = 0;
  const v = while (i < 3) : (i += 1) {
    if (i == 2) break i;
  } else 0;
  return v;
}"#
        ),
        vec![("f".into(), 3, 0, 4)]
    );
}

#[test]
fn abc_struct_methods_are_units() {
    assert_eq!(
        scores(
            r#"const S = struct {
  x: i32,
  pub fn get(self: *const S) i32 {
    return self.x + 1;
  }
};"#
        ),
        vec![("get".into(), 0, 1, 0)]
    );
}

#[test]
fn abc_test_block_is_a_unit() {
    let got = scores("test \"math\" {\n  const a = 1 + 2;\n  _ = a;\n}");
    assert_eq!(got, vec![("\"math\"".into(), 1, 1, 0)]);
}

#[test]
fn used_once_inline_candidate_for_pure_literal() {
    assert_eq!(
        used("fn f() i32 {\n  const dead = 5;\n  return dead;\n}"),
        vec!["dead"]
    );
}

#[test]
fn used_once_spares_impure_rhs_and_compound() {
    assert_eq!(
        used("fn f(b: i32) i32 {\n  var g = compute(b);\n  g += 1;\n  return g;\n}"),
        Vec::<String>::new()
    );
}

#[test]
fn used_once_params_and_payloads_are_protocol() {
    assert_eq!(
        used("fn f(p: i32, opt: ?i32) void {\n  p = 3;\n  if (opt) |v| { _ = v; }\n}"),
        Vec::<String>::new()
    );
}

#[test]
fn never_used_reports_locals_but_not_container_state() {
    assert_eq!(
        dead("const G = 1;\nfn m(q: i32) void {\n  const lost = 1;\n  _ = q;\n}"),
        vec!["lost"]
    );
}

#[test]
fn never_used_unused_parameter() {
    assert_eq!(dead("fn m(q: i32) void {\n}"), vec!["q"]);
}

#[test]
fn field_and_deref_writes_bind_nothing() {
    assert_eq!(
        dead("fn f(p: *i32) void {\n  p.* = 1;\n}"),
        Vec::<String>::new()
    );
}

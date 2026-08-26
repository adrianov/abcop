use crate::paths::{parse_file_lang, Lang};
use super::*;

fn scores(lang: Lang, code: &str, max: f64) -> Vec<AbcOffense> {
    let tree = crate::paths::parse_file_lang(code.as_bytes(), lang).unwrap();
    analyze(code.as_bytes(), &tree, lang, max)
}

#[test]
fn javascript_counts_assignments_calls_and_conditions() {
    let code = "\
function checkout(user, items) {
  let total = 0;
  for (const it of items) {
    total = total + it.price;
  }
  if (user.vip) {
    total = applyDiscount(total);
  }
  log(total);
}
";
    // Force all scores out for exact-vector assertions.
    let off = scores(Lang::Js, code, 0.0);
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].name, "checkout");
    // A: total init, total += price, total = applyDiscount(...)
    // B: applyDiscount call, log call, `+` arithmetic operator
    // C: for-of, if
    assert_eq!(off[0].vector, "<3, 3, 2>");
    assert!(scores(Lang::Js, code, 17.0).is_empty());
}

#[test]
fn ts_const_arrow_is_a_named_unit_and_nested_fns_do_not_double_count() {
    let off = scores(
        Lang::Ts,
        "\
const outer = (xs) => {
  function inner(y) {
    return helper(y) + helper(y);
  }
  return inner(1);
};
",
        0.0,
    );
    assert_eq!(off.len(), 2, "{off:?}");
    let inner = off.iter().find(|o| o.name == "inner").unwrap();
    let outer = off.iter().find(|o| o.name == "outer").unwrap();
    // Inner owns helper x2 (+ operator); outer only its own call.
    assert_eq!(inner.vector, "<0, 3, 0>");
    assert_eq!(outer.vector, "<0, 1, 0>");
}

#[test]
fn c_counts_loop_body_with_updates_and_comparisons() {
    let off = scores(
        Lang::C,
        "\
int sum_upto(int n) {
  int s = 0;
  for (int i = 0; i < n; i++) {
    s += i;
  }
  return s;
}
",
        0.0,
    );
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].name, "sum_upto");
    // A: s=0, i=0 init, s+=i, i++ update
    // B: none (comparisons are conditions here)
    // C: for, i<n comparison
    assert_eq!(off[0].vector, "<4, 0, 2>");
}

#[test]
fn objective_c_message_sends_are_branches() {
    let off = scores(
        Lang::ObjC,
        "\
@implementation Widget
- (int)pick:(NSArray *)items {
  if ([items count] > 0) {
    return [[self factory] build];
  }
  return 0;
}
@end
",
        0.0,
    );
    assert_eq!(off.len(), 1, "{off:?}");
    // B: [items count], [self factory], [.. build] = 3 sends
    // C: if, > = 2
    assert_eq!(off[0].vector, "<0, 3, 2>");
}

#[test]
fn swift_functions_score_on_the_same_scale() {
    let off = scores(
        Lang::Swift,
        "\
func grade(_ score: Int) -> String {
  if score >= 90 {
    return \"A\"
  } else if score >= 80 {
    return \"B\"
  }
  return \"F\"
}
",
        0.0,
    );
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].name, "grade");
    // C: two ifs + two >= comparisons
    assert_eq!(off[0].vector, "<0, 0, 4>");
}

#[test]
fn cpp_throw_and_new_are_branches() {
    let off = scores(
        Lang::Cpp,
        "\
Widget* make(int n) {
  if (n <= 0) throw std::invalid_argument(\"n\");
  return new Widget(n);
}
",
        0.0,
    );
    assert_eq!(off.len(), 1);
    // B: throw, new
    // C: if, <= comparison
    assert_eq!(off[0].vector, "<0, 2, 2>");
}

// ---- UsedOnce / NeverUsed over the JS/TS family ----

fn js_used(src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Js).expect("js parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree);
    let mut v: Vec<_> = super::used_once_offenses(&sc)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn js_dead(src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Js).expect("js parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree);
    let mut v: Vec<_> = super::never_used_offenses(&sc)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

#[test]
fn js_never_used_flags_dead_binding() {
    let src = "function f(items) {\n  const unused = items.length;\n  return 1;\n}";
    assert_eq!(js_dead(src), vec!["unused"]);
}

#[test]
fn js_member_reads_are_not_variable_reads() {
    // `it.length` reads `it`, never a binding named `length`
    let src = "function f(items) {\n  let n = 0;\n  for (const it of items) { n += it.length; }\n  return n;\n}";
    assert_eq!(js_dead(src), Vec::<String>::new());
}

#[test]
fn js_used_once_flags_inline_candidate() {
    let src = "function f(a, b) {\n  const sum = 2 * 21;\n  return sum;\n}";
    assert_eq!(js_used(src), vec!["sum"]);
}

#[test]
fn js_used_once_rejections() {
    let src = "function f(items) {\n               \x20 const a = helper();\n               \x20 let b = 1; b = 2;\n               \x20 let c = 1; c += 1;\n               \x20 if (items) { let d = 1; }\n               \x20 return a;\n}";
    assert_eq!(js_used(src), Vec::<String>::new());
}

#[test]
fn js_loop_heads_are_protocol() {
    let src = "function f(items) {\n  for (const k in items) { items[k]; }\n}";
    assert_eq!(js_dead(src), Vec::<String>::new());
}

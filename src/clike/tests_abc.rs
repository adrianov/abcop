//! AbcSize end-to-end vectors per language: every unit scores on one
//! shared scale (named declarations own their body; nested units never
//! double-count into the parent).

use super::*;
use crate::paths::Lang;

fn scores(lang: Lang, code: &str, max: f64) -> Vec<AbcOffense> {
    let tree = crate::paths::parse_file_lang(code.as_bytes(), lang).unwrap();
    all_scores(code.as_bytes(), &tree, lang)
        .into_iter()
        .filter(|o| o.score > max)
        .collect()
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

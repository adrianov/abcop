//! End-to-end assertions over the Dart backend: AbcSize vectors,

use super::{build, never_used_offenses, used_once_offenses};
use crate::abc::parse_vector;
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::dart::DartFile<'static> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Dart).expect("dart parses");
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
    // A: `once` declarator; B: call, additive; C: none
    assert_eq!(
        scores("int usedLater(int a) {\n  var once = a + compute(1);\n  return once;\n}"),
        vec![("usedLater".into(), 1, 2, 0)]
    );
}

#[test]
fn abc_class_members_named_and_scored() {
    // value getter: B only (binary); add: two calls (member call +
    // print); plain field write inside add() stays uncounted
    assert_eq!(
        scores(
            r#"class Cart {
  int items = 0;
  int get value => items * 2;
  void add() {
    items.add(this);
    print('n');
  }
}"#
        ),
        vec![("value".to_string(), 0, 1, 0), ("add".to_string(), 0, 2, 0)]
    );
}

#[test]
fn abc_constructors_take_signature_names() {
    // Foo ctor: B only (print); the bodyless `.named` header scores
    // nothing; factory arrow body holds the single Foo(0) call. The
    // initializer list must never hijack the name slot.
    assert_eq!(
        scores(
            r#"class Foo {
  int x;
  Foo(this.x) : x = 3 {
    print(x);
  }
  Foo.named(this.x);
  factory Foo.other() => Foo(0);
}"#
        ),
        vec![("Foo".to_string(), 0, 1, 0), ("other".to_string(), 0, 1, 0),]
    );
}

#[test]
fn abc_branch_and_guard_family_counts_c() {
    // expected: <6, 3, 8> -- see the add_special/add_table docs.
    let got = scores(
        r#"int complicated(bool flag) {
  int x = 0;
  if (x > 10 && flag) {
    x *= 2;
  }
  for (final i in [1, 2]) {
    x += i;
  }
  switch (x) {
    case 1:
      break;
    default:
      break;
  }
  try {
    x = risky(x);
  } on Exception catch (err) {
    print(err);
  }
  var pick = flag ? 'y' : 'n';
  print(pick);
  return x;
}"#,
    );
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, 6, "assignments");
    assert_eq!(got[0].2, 3, "calls");
    assert_eq!(got[0].3, 8, "branches/conditions");
}

#[test]
fn used_once_inline_candidate_for_pure_literal() {
    assert_eq!(
        used("int f() {\n  int dead = 5;\n  return dead;\n}"),
        vec!["dead"]
    );
}

#[test]
fn used_once_spares_impure_rhs_and_compound() {
    // compute() call fails purity; += rewrite is never a candidate
    assert_eq!(
        used("int f(int b) {\n  var g = compute(b);\n  g += 1;\n  return g;\n}"),
        Vec::<String>::new()
    );
}

#[test]
fn used_once_params_are_protocol() {
    // parameter rebinding produces no inline candidates
    assert_eq!(used("void f(int p) {\n  p = 3;\n}"), Vec::<String>::new());
}

#[test]
fn never_used_reports_locals_but_not_fields_or_protocol() {
    // lost: unused local. q: unused parameter. Field `f0` binds nothing.
    assert_eq!(
        dead("class K {\n  int f0 = 9;\n}\nint m(int q) {\n  var lost = 1;\n}"),
        vec!["lost", "q"]
    );
}

#[test]
fn never_used_interpolated_read_satisfies_use() {
    // '$n' registers a real read via identifier_dollar_escaped
    assert_eq!(
        dead("void show(int n) {\n  print('$n');\n}"),
        Vec::<String>::new()
    );
}

#[test]
fn closure_capture_of_outer_local_is_a_real_use() {
    // Dart units open Block scopes: the local fn reads the outer write
    assert_eq!(
        dead(
            "void outer() {\n  int captured = 1;\n  void inner() {\n    print(captured);\n  }\n  inner();\n}"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn pattern_destructured_unused_names_reported() {
    // [p, q] destructuring declares both; neither read afterwards
    assert_eq!(
        dead("void grab(List<int> xs) {\n  final [p, q] = xs;\n}"),
        vec!["p", "q"]
    );
}

#[test]
fn never_used_parameter_member_access_is_a_real_use() {
    assert_eq!(
        dead("class C {\n  int m(Obj dto) {\n    return dto.x;\n  }\n}"),
        Vec::<String>::new()
    );
    assert_eq!(
        dead("class C {\n  void m(Obj dto) {\n    print(dto.images.map((e) => e.x));\n  }\n}"),
        Vec::<String>::new()
    );
}

#[test]
fn never_used_factory_constructor_formals_are_protocol() {
    assert_eq!(
        dead(
            r#"@freezed
class E with _$E {
  const factory E.f({required int barcode}) = _F;
}"#,
        ),
        Vec::<String>::new()
    );
}

#[test]
fn never_used_constructor_formals_with_body_are_reported() {
    assert_eq!(
        dead("class Foo {\n  int x;\n  Foo(int spare, this.x) {\n    print(x);\n  }\n}"),
        vec!["spare"]
    );
}

#[test]
fn never_used_unimplemented_stub_formals_are_protocol() {
    assert_eq!(
        dead("abstract class M {\n  int toModel(int dto) => throw UnimplementedError();\n}"),
        Vec::<String>::new()
    );
}

#[test]
fn cascade_member_slots_do_not_shadow_reads() {
    // cascade property writes are instance state; obj itself is read
    assert_eq!(
        dead("void go(Obj obj) {\n  obj..m1()..f = 2;\n}"),
        Vec::<String>::new()
    );
}

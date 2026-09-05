//! UsedOnce / NeverUsed contract vectors for the Swift scope collector.

use crate::paths::{Lang, parse_file_lang};

fn used(lang: Lang, src: &'static str) -> Vec<String> {
    let mut v: Vec<_> = super::used_once_offenses(
        &super::collect_scopes(
            src.as_bytes(),
            &parse_file_lang(src.as_bytes(), lang).expect("fixture parses"),
            lang,
        ),
        lang,
    )
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn dead(lang: Lang, src: &'static str) -> Vec<String> {
    let mut v: Vec<_> = super::never_used_offenses(
        &super::collect_scopes(
            src.as_bytes(),
            &parse_file_lang(src.as_bytes(), lang).expect("fixture parses"),
            lang,
        ),
        lang,
    )
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

#[test]
fn swift_never_used_flags_dead_binding() {
    let src = "func f() {\n  let unused = 1\n  return 0\n}";
    assert_eq!(dead(Lang::Swift, src), vec!["unused".to_string()]);
}

#[test]
fn swift_used_once_flags_inline_candidate() {
    let src =
        "func f(a: Int) -> Int {\n  let r = a + 1\n  let d = r + 2\n  let g = 5\n  return g + d\n}";
    // `g` is assigned a pure literal and read once -> inline candidate.
    // `d`/`r` have local-reading (impure) RHS -> not candidates, matching
    // the JS backend purity rules.
    assert_eq!(used(Lang::Swift, src), vec!["g".to_string()]);
}
#[test]
fn swift_reassigned_var_is_not_inline_candidate() {
    let src = "func f() {\n  var c = 0\n  c = 1\n  c = 2\n  return c\n}";
    assert_eq!(used(Lang::Swift, src), Vec::<String>::new());
}

#[test]
fn swift_immediate_call_chain_yes_intervening_and_loop_no() {
    assert_eq!(
        used(
            Lang::Swift,
            "func f() -> Int {\n  let a = helper()\n  return a\n}"
        ),
        vec!["a"]
    );
    assert_eq!(
        used(
            Lang::Swift,
            "func f() -> Int {\n  let a = helper()\n  side()\n  return a\n}"
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        used(
            Lang::Swift,
            "func f(_ items: [Int]) {\n  let a = helper()\n  for x in items { use(a) }\n}"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn swift_dead_call_keeps_initializer() {
    let src = b"func f() -> Int {\n  let gone = helper()\n  return 1\n}";
    let f = super::never_used_offenses(
        &super::collect_scopes(
            src,
            &parse_file_lang(src, Lang::Swift).unwrap(),
            Lang::Swift,
        ),
        Lang::Swift,
    );
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "gone");
    assert!(f[0].keep_init);
}

#[test]
fn swift_member_reads_are_not_variable_reads() {
    let src = "class C {\n  func f() {\n    let x = 1\n    return self.helper + x\n  }\n  func helper() -> Int { 0 }\n}";

    let bindings: Vec<String> = super::collect_scopes(
        src.as_bytes(),
        &parse_file_lang(src.as_bytes(), Lang::Swift).unwrap(),
        Lang::Swift,
    )
        .scopes
        .iter()
        .flat_map(|s| s.entries.keys())
        .map(|k| k.as_ref().to_string())
        .collect();
    // `x` is a local binding (read via the trailing expression); `helper`
    // is a member read off `self` and must NOT appear as a local binding.
    assert!(bindings.contains(&"x".to_string()));
    assert!(!bindings.contains(&"helper".to_string()));
}

#[test]
fn swift_member_field_read_does_not_count_as_local_read() {
    // `self.x` must not register a phantom read of a same-named local `x`;
    // the local is never read, so it is NeverUsed (not a UsedOnce candidate).
    let src = "class C {\n  func f() -> Int {\n    let x = 1\n    return self.x\n  }\n  var x: Int { 0 }\n}";
    assert_eq!(dead(Lang::Swift, src), vec!["x".to_string()]);
    assert_eq!(used(Lang::Swift, src), Vec::<String>::new());
}

#[test]
fn swift_compound_assignment_is_not_used_once() {
    let src = "func f() {\n  var t = 0\n  t += 1\n  return t\n}";
    assert_eq!(used(Lang::Swift, src), Vec::<String>::new());
}

#[test]
fn swift_closure_rhs_is_not_pure() {
    // A closure is not a pure RHS, so even a single-use captured local
    // must not be inlined across the capture boundary.
    let src = "func f() {\n  let n = 1\n  let c = { (_: Int) in n }\n  return c(0) + n\n}";
    assert_eq!(used(Lang::Swift, src), Vec::<String>::new());
}

#[test]
fn swift_type_members_are_not_locals() {
    // Stored / computed / @State members under class_body must not be
    // UsedOnce or NeverUsed; only nested function locals are.
    let src = "struct V: View {\n  var projects: [Int]\n  @State private var text: String = \"\"\n  var body: some View {\n    Text(text)\n  }\n  func f() {\n    let unused = 1\n    let once = 2\n    return once + projects.count\n  }\n}";
    assert_eq!(dead(Lang::Swift, src), vec!["unused".to_string()]);
    assert_eq!(used(Lang::Swift, src), vec!["once".to_string()]);
}

#[test]
fn swift_computed_property_locals_are_tracked() {
    // Locals inside `@computed_value` must bind and resolve reads; the
    // computed property name itself is a type member and stays quiet.
    let src = "struct S {\n  var body: Int {\n    let dead = 1\n    let once = 2\n    return once\n  }\n}";
    assert_eq!(dead(Lang::Swift, src), vec!["dead".to_string()]);
    assert_eq!(used(Lang::Swift, src), vec!["once".to_string()]);
}

#[test]
fn swift_lambda_literal_captures_outer_local() {
    // tree-sitter-swift uses `lambda_literal` (not `closure_expression`);
    // a read inside the trailing closure must count toward the outer local.
    let src = "func f(_ items: [Int]) {\n  let a = 1\n  items.forEach { _ in print(a) }\n}";
    assert_eq!(dead(Lang::Swift, src), Vec::<String>::new());
    assert_eq!(used(Lang::Swift, src), vec!["a".to_string()]);
}

#[test]
fn swift_for_in_collection_counts_as_read() {
    // `for x in ids` must record a read of `ids` (and not treat the loop
    // binder as a local introduction).
    let src = "func f(_ items: [Int]) {\n  let ids = items\n  for x in ids { print(x) }\n}";
    assert_eq!(dead(Lang::Swift, src), Vec::<String>::new());
    assert_eq!(used(Lang::Swift, src), vec!["ids".to_string()]);
}

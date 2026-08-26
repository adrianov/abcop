//! UsedOnce / NeverUsed contract vectors for the Swift scope collector.

use crate::paths::{parse_file_lang, Lang};

fn used(lang: Lang, src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), lang).expect("fixture parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree, lang);
    let mut v: Vec<_> = super::used_once_offenses(&sc, lang)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn dead(lang: Lang, src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), lang).expect("fixture parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree, lang);
    let mut v: Vec<_> = super::never_used_offenses(&sc, lang)
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
    let src = "func f(a: Int) -> Int {\n  let r = a + 1\n  let d = r + 2\n  let g = 5\n  return g + d\n}";
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
fn swift_member_reads_are_not_variable_reads() {
    let src = "class C {\n  func f() {\n    let x = 1\n    return self.helper + x\n  }\n  func helper() -> Int { 0 }\n}";
    let tree = parse_file_lang(src.as_bytes(), Lang::Swift).unwrap();
    let sc = super::collect_scopes(src.as_bytes(), &tree, Lang::Swift);
    let bindings: Vec<String> = sc.scopes.iter().flat_map(|s| s.entries.keys()).map(|k| k.as_ref().to_string()).collect();
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


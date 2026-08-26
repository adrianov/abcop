//! UsedOnce / NeverUsed end-to-end vectors over the JS/TS and Swift
//! scope-model backends. AbcSize vectors live in [`tests_abc`].

use crate::paths::{parse_file_lang, Lang};

// ---- UsedOnce / NeverUsed over the JS/TS family ----
fn js_used(src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Js).expect("js parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree, Lang::Js);
    let mut v: Vec<_> = super::used_once_offenses(&sc, Lang::Js)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn js_dead(src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Js).expect("js parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree, Lang::Js);
    let mut v: Vec<_> = super::never_used_offenses(&sc, Lang::Js)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn swift_used(src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Swift).expect("swift parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree, Lang::Swift);
    let mut v: Vec<_> = super::used_once_offenses(&sc, Lang::Swift)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn swift_dead(src: &'static str) -> Vec<String> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Swift).expect("swift parses");
    let sc = super::collect_scopes(src.as_bytes(), &tree, Lang::Swift);
    let mut v: Vec<_> = super::never_used_offenses(&sc, Lang::Swift)
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

#[test]
fn js_exported_functions_are_analyzed() {
    // export wrappers must not swallow the declaration
    let src = "export function f(items) {\n  const unused = items.length;\n  return 1;\n}";
    assert_eq!(js_dead(src), vec!["unused"]);
}

#[test]
fn js_class_methods_track_locals() {
    let src = "class C {\n  m(items) {\n    const unused = items.length;\n    return 1;\n  }\n}";
    assert_eq!(js_dead(src), vec!["unused"]);
}

#[test]
fn js_closures_read_module_bindings() {
    // the tetrogrow server.js shape: module-level const read from a
    // nested function/closure must resolve
    let src = "const diagLog = build();
function wire(h) {
  try { append(diagLog, h); } catch {}
}
function build(){return 1}";
    assert_eq!(js_dead(src), Vec::<String>::new());
}

#[test]
fn js_destructuring_declarations_bind_elements() {
    let src = "class B {
  steer(d) { return [1, 2]; }
  tick() {
    const [rx, ry] = this.steer(1);
    return rx + ry;
  }
}";
    assert_eq!(js_dead(src), Vec::<String>::new());
}

#[test]
fn js_object_pattern_shorthand_binds() {
    let src = "function controlHuman(p) {
  const { ix, iy } = p.input;
  return ix + iy;
}";
    assert_eq!(js_dead(src), Vec::<String>::new());
}

#[test]
fn js_shorthand_object_literal_is_a_read() {
    let src = "function emit(diagLog, insRun) {\n\x20 return { diagLog, insRun };\n}";
    assert_eq!(js_dead(src), Vec::<String>::new());
}

// ---- UsedOnce / NeverUsed over Swift ----

#[test]
fn swift_never_used_flags_dead_binding() {
    let src = "func f() {\n  let unused = 1\n  return 0\n}";
    assert_eq!(swift_dead(src), vec!["unused".to_string()]);
}

#[test]
fn swift_used_once_flags_inline_candidate() {
    let src = "func f(a: Int) -> Int {\n  let r = a + 1\n  let d = r + 2\n  let g = 5\n  return g + d\n}";
    // `g` is assigned a pure literal and read once -> inline candidate.
    // `d`/`r` have local-reading (impure) RHS -> not candidates, matching
    // the JS backend purity rules.
    assert_eq!(swift_used(src), vec!["g".to_string()]);
}
#[test]
fn swift_reassigned_var_is_not_inline_candidate() {
    let src = "func f() {\n  var c = 0\n  c = 1\n  c = 2\n  return c\n}";
    assert_eq!(swift_used(src), Vec::<String>::new());
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
    assert_eq!(swift_dead(src), vec!["x".to_string()]);
    assert_eq!(swift_used(src), Vec::<String>::new());
}

#[test]
fn swift_compound_assignment_is_not_used_once() {
    let src = "func f() {\n  var t = 0\n  t += 1\n  return t\n}";
    assert_eq!(swift_used(src), Vec::<String>::new());
}

#[test]
fn swift_closure_rhs_is_not_pure() {
    // A closure is not a pure RHS, so even a single-use captured local
    // must not be inlined across the capture boundary.
    let src = "func f() {\n  let n = 1\n  let c = { (_: Int) in n }\n  return c(0) + n\n}";
    assert_eq!(swift_used(src), Vec::<String>::new());
}


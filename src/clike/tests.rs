//! UsedOnce / NeverUsed contract vectors for the JavaScript/TypeScript
//! scope collector. Swift vectors live in [`tests_swift`], the plain-C
//! family in [`tests_cfamily`], and AbcSize score vectors in
//! [`tests_abc`].

use crate::paths::{Lang, parse_file_lang};

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
fn js_never_used_flags_dead_binding() {
    let src = "function f(items) {\n  const unused = items.length;\n  return 1;\n}";
    assert_eq!(dead(Lang::Js, src), vec!["unused"]);
}

#[test]
fn js_member_reads_are_not_variable_reads() {
    // `it.length` reads `it`, never a binding named `length`
    let src = "function f(items) {\n  let n = 0;\n  for (const it of items) { n += it.length; }\n  return n;\n}";
    assert_eq!(dead(Lang::Js, src), Vec::<String>::new());
}

#[test]
fn js_used_once_flags_inline_candidate() {
    let src = "function f(a, b) {\n  const sum = 2 * 21;\n  return sum;\n}";
    assert_eq!(used(Lang::Js, src), vec!["sum"]);
}

#[test]
fn js_used_once_rejections() {
    let src = "function f(items) {\n               \x20 const a = helper();\n               \x20 let b = 1; b = 2;\n               \x20 let c = 1; c += 1;\n               \x20 if (items) { let d = 1; }\n               \x20 return a;\n}";
    assert_eq!(used(Lang::Js, src), Vec::<String>::new());
}

#[test]
fn js_loop_heads_are_protocol() {
    let src = "function f(items) {\n  for (const k in items) { items[k]; }\n}";
    assert_eq!(dead(Lang::Js, src), Vec::<String>::new());
}

#[test]
fn js_exported_functions_are_analyzed() {
    // export wrappers must not swallow the declaration
    let src = "export function f(items) {\n  const unused = items.length;\n  return 1;\n}";
    assert_eq!(dead(Lang::Js, src), vec!["unused"]);
}

#[test]
fn js_class_methods_track_locals() {
    let src = "class C {\n  m(items) {\n    const unused = items.length;\n    return 1;\n  }\n}";
    assert_eq!(dead(Lang::Js, src), vec!["unused"]);
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
    assert_eq!(dead(Lang::Js, src), Vec::<String>::new());
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
    assert_eq!(dead(Lang::Js, src), Vec::<String>::new());
}

#[test]
fn js_object_pattern_shorthand_binds() {
    let src = "function controlHuman(p) {
  const { ix, iy } = p.input;
  return ix + iy;
}";
    assert_eq!(dead(Lang::Js, src), Vec::<String>::new());
}

#[test]
fn js_shorthand_object_literal_is_a_read() {
    let src = "function emit(diagLog, insRun) {\n\x20 return { diagLog, insRun };\n}";
    assert_eq!(dead(Lang::Js, src), Vec::<String>::new());
}

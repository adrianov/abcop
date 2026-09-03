//! UsedOnce / NeverUsed contract vectors for the plain-C-family scope
//! collectors (C, C++, Objective-C).

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

// ---- UsedOnce / NeverUsed over C / C++ / Objective-C ----
// One vector per dispatcher shape: dead binding, pure inline candidate,
// compound/`++` rejection, member-slot distinctness, loop-head protocol.

#[test]
fn c_flags_dead_binding_and_pure_inline_candidate() {
    let src = "int main(void) {\n  int dead = 1;\n  int cand = 2 + 3;\n  return cand + 0;\n}";
    assert_eq!(dead(Lang::C, src), vec!["dead".to_string()]);
    assert_eq!(used(Lang::C, src), vec!["cand".to_string()]);
}

#[test]
fn c_pointer_wrappers_still_bind_the_name() {
    // `char *p = s;`: pointer_declarator wraps the identifier; bare alias.
    let src = "void f(char *s) {\n  char *p = s;\n  use(p);\n}";
    assert_eq!(used(Lang::C, src), vec!["p"]);
}

#[test]
fn c_compound_ops_and_increments_are_not_inline_candidates() {
    let src = "int main(void) {\n  int t = 0;\n  t += 1;\n  t++;\n  return t;\n}";
    assert_eq!(used(Lang::C, src), Vec::<String>::new());
}

#[test]
fn c_loop_head_vars_are_protocol() {
    // `i` is written once and read once inside the for -- the loop head
    // must be excluded so no inlining suggestion fires on it.
    let src = "int main(void) {\n  int total = 0;\n  for (int i = 0; i < 3; i++) { total += i; }\n  return total;\n}";
    assert_eq!(used(Lang::C, src), Vec::<String>::new());
    assert_eq!(dead(Lang::C, src), Vec::<String>::new());
}

#[test]
fn c_globals_are_outside_local_analysis_and_object_writes_read_the_base() {
    // file-scope statics/enums live beyond single-file scope: reads of
    // them resolve to nothing. Writing `p.px` also *reads* `p` itself --
    // the object is evaluated -- so `p` stays clean despite the field write.
    let src = "struct P { int px; };\nstatic int g = 4;\nint main(void) {\n  int orphan;\n  struct P p;\n  p.px = g;\n  return g;\n}";
    assert_eq!(used(Lang::C, src), Vec::<String>::new());
    assert_eq!(dead(Lang::C, src), vec!["orphan".to_string()]);
}
#[test]
fn cpp_pure_literal_flags_while_lambda_and_field_reads_do_not() {
    // the lambda is an impure RHS (never a candidate); `local` is a true
    // inline suggestion; `width()` calls stay member slots, not locals
    let src = "class W { public:\n  int width() { return f; }\n  int f;\n };\nint main() {\n  auto lam = [](){ return 1; };\n  W wt;\n  int local = 3;\n  return lam() + wt.width() + local;\n}";
    assert_eq!(used(Lang::Cpp, src), vec!["local".to_string()]);
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
}

#[test]
fn header_extension_uses_cpp_so_class_fields_stay_clean() {
    // `.h` must not pick the C grammar: that misreads `class` bodies as
    // functions and NeverUsed-flags every member.
    assert_eq!(
        crate::paths::lang_for(std::path::Path::new("MainWindowPrivate.h")),
        Lang::Cpp
    );
    let src = "class MainWindowPrivate {\npublic:\n  int life;\n  QWidget *presence;\n  Arena *arena = nullptr;\n};\n";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
}

#[test]
fn export_macro_class_is_not_a_function_of_locals() {
    // `class UTIL_EXPORT Foo { ... }` is misparsed as a function_definition;
    // members (and a phantom empty declarator on `class Impl;`) must not
    // surface as NeverUsed.
    let src = "class UTIL_EXPORT SaxParser {\nprivate:\n  class Impl;\n  std::unique_ptr<Impl> m_impl;\nprotected:\n  bool m_processed { true };\n};\n";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
    assert_eq!(used(Lang::Cpp, src), Vec::<String>::new());
}

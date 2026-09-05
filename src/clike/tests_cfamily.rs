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
fn c_immediate_call_chain_yes_intervening_and_loop_no() {
    assert_eq!(
        used(Lang::C, "int f(void) {\n  int a = helper();\n  return a;\n}"),
        vec!["a"]
    );
    assert_eq!(
        used(
            Lang::C,
            "int f(void) {\n  int a = helper();\n  side();\n  return a;\n}"
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        used(
            Lang::C,
            "int f(int n) {\n  int a = helper();\n  for (int i = 0; i < n; i++) { use(a); }\n  return 0;\n}"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn c_dead_call_keeps_initializer() {
    let src = b"int f(void) {\n  int gone = helper();\n  return 1;\n}";
    let f = super::never_used_offenses(
        &super::collect_scopes(src, &parse_file_lang(src, Lang::C).unwrap(), Lang::C),
        Lang::C,
    );
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "gone");
    assert!(f[0].keep_init);
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

#[test]
fn cpp_ifdef_else_if_does_not_bind_if_keyword() {
    // Preprocessor-split `if` / `else if` parses `if` as a declarator;
    // keyword bind filter must refuse it (ifdef bodies are walked for
    // real reads). Keep `dead` before the broken `else` — that misparse
    // can close the function early in the CST.
    let src = "void f(int sock) {\n  int dead = 1;\n  if (sock == 1) { return; }\n#ifdef WITH_UTP\n  else if (sock == 2) { return; }\n#endif\n  else { return; }\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_raii_lifetime_guards_are_not_never_used() {
    let src = "void f(Session* session, std::mutex& m, addrinfo* info, QMutex* qm, CriticalSection& cs) {\n  auto const lock = session->unique_lock();\n  auto const held = cm->lock();\n  std::lock_guard const guard{ m };\n  QMutexLocker qlock(qm);\n  HashManager::HashPauser pauser;\n  Lock l(cs);\n  FastLock fl(cs);\n  File f(target, File::WRITE, File::CREATE);\n  auto const keep_alive = shared_from_this();\n  auto const blocker = QSignalBlocker{ w };\n  auto const info_uniq = std::unique_ptr<addrinfo, decltype(&freeaddrinfo)>{ info, freeaddrinfo };\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_block_ctor_style_args_count_as_reads() {
    // `QFile f(path)` parses as function_declarator; params are expression args.
    let src = "void g() {\n  QString path = x();\n  QFile f(path);\n  (void)f;\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_qt_emit_and_debug_block_are_not_locals() {
    // `emit sig(x)` and `DEBUG_BLOCK\\n call()` parse as declarations whose
    // type is the macro; refuse the bind and still count arg reads. A single
    // use of `status` via emit is UsedOnce (real), not NeverUsed (the old FP).
    let src = "void f(Hubs& hubs) {\n  auto it = hubs.find(url);\n  if (it != hubs.end()) {\n    emit hubUnregistered(it.value());\n    hubs.erase(it);\n  }\n  QString status = tr(\"hi\");\n  emit coreConnecting(status);\n  DEBUG_BLOCK\n  setAcceptDrops(true);\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
    assert_eq!(used(Lang::Cpp, src), vec!["status".to_string()]);
}

#[test]
fn cpp_ifdef_orphaned_else_does_not_bind_following_name() {
    // `#ifdef` between `if` and `else` makes `else stmt` a declaration with
    // type `else` — do not bind `notify` / `fprintf` as locals.
    let src = "void f(Module* notify) {\n  if (t == QtNotify)\n    notify = new A();\n#ifdef DBUS\n  else\n    notify = new B();\n#endif\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_adjacent_macros_and_index_assign_are_not_locals() {
    // Error recovery: `paths[I] = a + "x" MACRO MACRO` splits into a fake
    // structured_binding decl and `MACRO MACRO;`.
    let src = "void f(string* paths) {\n  paths[PATH_LOCALE] = linExecutablePath() + \"/../../\" LOCALE_DIR PATH_SEPARATOR_STR;\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_template_arg_reads_constexpr_local() {
    // Non-type template arguments are `type_identifier` in tree-sitter-cpp;
    // counting them as reads clears the NeverUsed false positive on BufSize.
    let src = "template<std::size_t N, class T> struct StackBuffer { T data[N]; };\nvoid f() {\n  static auto constexpr BufSize = 32U;\n  auto outbuf = StackBuffer<BufSize, char>{};\n  (void)outbuf;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
    assert_eq!(used(Lang::Cpp, src), vec!["BufSize".to_string()]);
}

#[test]
fn cpp_nullability_macro_is_not_a_local() {
    let src = "void f(void) {\n  NS_ASSUME_NONNULL_END;\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_pointer_const_declarator_still_walks_rhs_reads() {
    // `* const ptr` must resolve via @declarator, not named_child(0) (the
    // const qualifier), or the RHS is dropped and outer locals look dead.
    let src = "bool f(char const* address) {\n  auto native = std::string{};\n  auto const* const addr = std::empty(native) ? address : native.c_str();\n  return addr != nullptr;\n}";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
}

#[test]
fn cpp_for_init_reads_outer_locals_without_binding_loop_var() {
    let src = "void f() {\n  auto const& spans = getSpans();\n  for (auto it = spans.rbegin(); it != spans.rend(); ++it) { use(*it); }\n}\n";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
    assert_eq!(used(Lang::Cpp, src), Vec::<String>::new());
}

#[test]
fn cpp_if_init_condition_counts_as_a_use() {
    let src = "void f(Task* task) {\n  if (auto const range = task->range()) { (void)0; }\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

#[test]
fn cpp_if_init_pointer_rhs_reads_outer_locals() {
    // `if (T* x = call(priv))` puts `@value` on the declaration (no
    // init_declarator); args must still count as reads.
    let src = "bool f() {\n  const bool priv = isPrivate();\n  if (OnlineUser* ou = findBest(priv)) { return true; }\n  return false;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
}

#[test]
fn cpp_reads_inside_ifdef_still_count() {
    let src = "void f() {\n  auto const abs = path();\n#if defined(A)\n  use(abs);\n#endif\n}\n";
    assert_eq!(dead(Lang::Cpp, src), Vec::<String>::new());
}

#[test]
fn cpp_reference_alias_written_later_is_not_never_used() {
    let src = "void f(int& a, int& b, int dir) {\n  auto& tgt = dir == 0 ? a : b;\n  tgt = 1;\n  int dead = 1;\n}\n";
    assert_eq!(dead(Lang::Cpp, src), vec!["dead".to_string()]);
}

use super::*;
use crate::abc::AbcOffense;
use crate::never_used::NeverUsedOffense;
use crate::used_once::UsedOnceOffense;

fn build_str(src: &str) -> RustFile<'_> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("rust grammar");

    build(
        src.as_bytes(),
        parser.parse(src, None).expect("syntax tree"),
    )
}

fn scores(src: &str) -> Vec<AbcOffense> {
    all_scores(&build_str(src))
}

fn flags(src: &str) -> Vec<UsedOnceOffense> {
    used_once_offenses(&build_str(src))
}

fn never_flags(src: &str) -> Vec<NeverUsedOffense> {
    never_used_offenses(&build_str(src))
}

#[test]
fn compute_method_vector() {
    let s = scores(
        "fn compute(items: &[Option<u32>], factor: u32) -> u32 {\n\
         \x20   let mut total = 0u32;\n\
         \x20   for item in items.iter() {\n\
         \x20       if item.is_none() {\n\
         \x20           continue;\n\
         \x20       }\n\
         \x20       total += item.unwrap() * factor;\n\
         \x20   }\n\
         \x20   total / factor\n\
         }",
    );
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].name, "compute");
    assert_eq!(s[0].vector, "<3, 5, 2>");
    assert!((s[0].score - 6.16).abs() < 1e-9);
}

#[test]
fn if_else_if_chain_conditions_without_else_bonus() {
    let s = scores(
        "fn cond(x: i32) -> i32 {\n\
         \x20   if x == 1 && x < 5 { 1 } else if x > 10 { 2 } else { 3 }\n\
         }",
    );
    assert_eq!(s[0].vector, "<0, 0, 6>");
}

#[test]
fn match_arms_and_guards() {
    let s = scores(
        "fn mat(c: u8) -> &'static str {\n\
         \x20   match c {\n\
         \x20       0 => \"zero\",\n\
         \x20       n if n > 10 => \"big\",\n\
         \x20       _ => \"other\",\n\
         \x20   }\n\
         }",
    );
    // three arms + guard comparison; binder `n` is one assignment
    assert_eq!(s[0].vector, "<1, 0, 4>");
}

#[test]
fn closures_roll_into_enclosing_function() {
    let s = scores(
        "fn closures(v: Vec<u32>) -> u32 {\n\
         \x20   let add = |a: u32| a + 1;\n\
         \x20   add(v.len() as u32)\n\
         }",
    );
    // A: add-let + closure param a; B: v.len + + binary + add call
    assert_eq!(s[0].vector, "<2, 3, 0>");
}

#[test]
fn macro_invocations_are_branches_and_token_reads_count() {
    let s = scores(
        "fn macros(n: u32) {\n\
         \x20   println!(\"{}\", n);\n\
         }",
    );
    assert_eq!(s[0].vector, "<0, 1, 0>");

    let s = scores(
        "fn macros2(n: u32) {\n\
         \x20   let m = n + 1;\n\
         \x20   println!(\"{}\", m);\n\
         }",
    );
    assert_eq!(s[0].vector, "<1, 2, 0>");
}

#[test]
fn try_operator_is_a_branch() {
    let s = scores(
        "fn try_op(x: Result<u32, ()>) -> Result<u32, ()> {\n\
         \x20   let y = x?;\n\
         \x20   Ok(y + 1)\n\
         }",
    );
    assert_eq!(s[0].vector, "<1, 3, 0>");
}

#[test]
fn shadowing_counts_as_multiple_writes_not_candidates() {
    let f = flags("fn f() {\n  let n = 1;\n  let n = n + 1;\n}");
    assert!(f.is_empty());
    let s = scores("fn f() {\n  let mut n = 1;\n  n += 1;\n  let n = n * 2;\n}");
    assert_eq!(s[0].vector, "<3, 1, 0>");
}

#[test]
fn simple_single_use_is_flagged_at_let_line() {
    let f = flags("fn f() {\n  let tmp = 42;\n  p(tmp);\n}");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "tmp");
    assert_eq!(f[0].line, 2);
    assert_eq!(f[0].column, 6);
}

#[test]
fn method_call_rhs_is_flagged() {
    let f = flags("fn f(x: &str) {\n  let s = x.to_string();\n  p(s);\n}");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "s");
}

#[test]
fn call_chain_rejected_with_intervening_statement() {
    let f = flags("fn f() {\n  let s = compute();\n  side();\n  p(s);\n}");
    assert!(f.is_empty(), "effectful RHS must not cross statements: {f:?}");
}

#[test]
fn call_chain_in_loop_read_rejected() {
    let f = flags("fn f(items: &[u8]) {\n  let s = compute();\n  for _ in items { p(s); }\n}");
    assert!(f.is_empty(), "read inside loop must not inline calls: {f:?}");
}

#[test]
fn dead_call_chain_keeps_initializer() {
    let f = never_flags("fn f() {\n  let gone = compute();\n}");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "gone");
    assert!(f[0].keep_init);
}

#[test]
fn second_read_rejected() {
    let f = flags("fn f() {\n  let t = 7;\n  p(t); p(t);\n}");
    assert!(f.is_empty());
}

#[test]
fn if_let_binding_never_candidate() {
    let f = flags("fn g(o: Option<u32>) {\n  if let Some(v) = o {\n    p(v);\n  }\n}");
    assert!(f.is_empty());
}

#[test]
fn tuple_and_struct_let_patterns_never_candidates() {
    let f = flags(
        "fn f() {\n  let (a, b) = (1, 2);\n  p(a + b);\n  let Some(x) = Some(3) else { return };\n  p(x);\n}",
    );
    assert!(f.is_empty(), "pattern lets are not inlineable: {f:?}");
}

#[test]
fn self_reborrow_keeps_binding() {
    let f = flags(
        "impl K {\n  fn f(&mut self) {\n    let s = self.scope_for();\n    self.declare(s);\n    let t = self.model().open();\n    self.use_t(t);\n  }\n  fn scope_for(&mut self) -> usize { 0 }\n  fn declare(&mut self, _s: usize) {}\n  fn model(&mut self) -> K { K }\n  fn open(&mut self) -> usize { 0 }\n  fn use_t(&mut self, _t: usize) {}\n}",
    );
    assert!(f.is_empty(), "mut self reborrow cannot inline: {f:?}");
}

#[test]
fn param_reborrow_keeps_binding() {
    let f = flags(
        "fn walk(b: &mut K, n: u32) {\n  let s = b.open_scope();\n  use_both(b, s);\n}\nfn use_both(_b: &mut K, _s: usize) {}\nimpl K {\n  fn open_scope(&mut self) -> usize { 0 }\n}",
    );
    assert!(f.is_empty(), "receiver reborrow cannot inline: {f:?}");
}

#[test]
fn refcell_borrow_into_let_else_keeps_guard() {
    // Inlining `cell.borrow().as_ref()` into let-else drops the Ref (E0716).
    let f = flags(
        "fn f(cell: &std::cell::RefCell<Option<String>>) -> Option<usize> {\n\
         \x20   let g = cell.borrow();\n\
         \x20   let Some(b) = g.as_ref() else { return None; };\n\
         \x20   Some(b.len())\n\
         }",
    );
    assert!(
        f.iter().all(|o| o.name != "g"),
        "RefCell guard must stay: {f:?}"
    );
}

#[test]
fn osstring_lossy_into_let_keeps_owner() {
    // `file_name().to_string_lossy()` binds a Cow that borrows the temporary.
    let f = flags(
        "fn f(e: std::fs::DirEntry) -> bool {\n\
         \x20   let n = e.file_name();\n\
         \x20   let s = n.to_string_lossy();\n\
         \x20   s.starts_with(\"lib\")\n\
         }",
    );
    assert!(
        f.iter().all(|o| o.name != "n"),
        "OsString owner must stay: {f:?}"
    );
}

#[test]
fn borrow_guard_rhs_keeps_binding_even_as_arg() {
    // Guard RHS is kept unconditionally — safer than chasing every E0716 shape.
    let f = flags(
        "fn f(cell: &std::cell::RefCell<u32>) {\n\
         \x20   let g = cell.borrow();\n\
         \x20   use_ref(g.as_ref());\n\
         }\n\
         fn use_ref(_: &u32) {}",
    );
    assert!(
        f.iter().all(|o| o.name != "g"),
        "RefCell borrow RHS must stay: {f:?}"
    );
}

#[test]
fn borrow_then_clone_rhs_keeps_binding() {
    let f = flags(
        "fn f(cell: &std::cell::RefCell<String>) {\n\
         \x20   let s = cell.borrow().clone();\n\
         \x20   use_s(s);\n\
         }\n\
         fn use_s(_: String) {}",
    );
    assert!(
        f.iter().all(|o| o.name != "s"),
        "borrow().clone() RHS must stay: {f:?}"
    );
}

#[test]
fn mutex_lock_unwrap_rhs_keeps_binding() {
    let f = flags(
        "fn f(m: &std::sync::Mutex<u32>) {\n\
         \x20   let g = m.lock().unwrap();\n\
         \x20   use_g(*g);\n\
         }\n\
         fn use_g(_: u32) {}",
    );
    assert!(
        f.iter().all(|o| o.name != "g"),
        "lock().unwrap() RHS must stay: {f:?}"
    );
}

#[test]
fn owned_alias_as_str_into_let_still_flagged() {
    // Alias of an owned value is not a temporary — inlining stays valid.
    let f = flags(
        "fn f(s: String) {\n\
         \x20   let t = s;\n\
         \x20   let u = t.as_str();\n\
         \x20   use_u(u);\n\
         }\n\
         fn use_u(_: &str) {}",
    );
    assert!(
        f.iter().any(|o| o.name == "t"),
        "owned alias must still inline: {f:?}"
    );
}

#[test]
fn match_to_string_lossy_keeps_owner() {
    let f = flags(
        "fn f(e: std::fs::DirEntry) {\n\
         \x20   let n = e.file_name();\n\
         \x20   match n.to_string_lossy() {\n\
         \x20       s if s.starts_with(\"lib\") => {}\n\
         \x20       _ => {}\n\
         \x20   }\n\
         }",
    );
    assert!(
        f.iter().all(|o| o.name != "n"),
        "match scrutinee borrow must keep owner: {f:?}"
    );
}

#[test]
fn non_guard_call_as_arg_still_flagged() {
    let f = flags(
        "fn f(x: &str) {\n\
         \x20   let s = x.to_string();\n\
         \x20   use_s(s.as_str());\n\
         }\n\
         fn use_s(_: &str) {}",
    );
    assert_eq!(f.iter().filter(|o| o.name == "s").count(), 1, "{f:?}");
}

#[test]
fn read_inside_later_closure_is_candidate() {
    let f = flags("fn k() {\n  let x = 42;\n  run(|| p(x));\n}");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "x");
}

#[test]
fn pure_composite_rhs_accepted() {
    let f = flags("fn f(a: u32, b: u32) {\n  let m = a * b + 1;\n  p(m);\n}");
    // a and b are params (Binding), rhs references them -> identifiers are
    // not pure per spec... `a * b + 1` contains identifier reads, so the
    // conservative purity gate rejects it.
    assert!(f.is_empty());
}

#[test]
fn rust_dead_let_is_flagged() {
    let f = never_flags("fn f() {\n  let gone = 5;\n}");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "gone");
    assert_eq!(f[0].line, 2);
}

#[test]
fn rust_shadow_chain_with_final_read_ok() {
    let f = never_flags("fn f() {\n  let n = 1;\n  let n = n + 1;\n  p(n);\n}");
    assert!(f.is_empty());
}

#[test]
fn rust_read_inside_macro_counts_as_use() {
    let f = never_flags("fn f() {\n  let v = 3;\n  println!(\"{}\", v);\n}");
    assert!(f.is_empty());
}

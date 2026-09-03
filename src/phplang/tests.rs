//! End-to-end assertions over the PHP backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::abc::parse_vector;
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::phplang::PhpFile<'static> {
    build(
        src.as_bytes(),
        parse_file_lang(src.as_bytes(), Lang::Php).expect("php parses"),
    )
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
fn assignment_and_binary_count_as_a_and_b() {
    assert_eq!(
        scores("<?php\nfunction simple($a) {\n  $x = $a + 1;\n  return $x;\n}"),
        vec![("simple".to_string(), 1, 1, 0)]
    );
}

#[test]
fn branches_loops_cases_tally_c() {
    // A: total, i+it heads, += , -= = 5
    // B: strlen call, unary minus of -5 = 2
    // C: foreach, if, >, ||, ===, elseif, ===, case, default = 9
    let src = "<?php\nfunction branchy(array $items, int $limit): int {\n\
               \x20 $total = 0;\n\
               \x20 foreach ($items as $i => $it) {\n\
               \x20   if ($i > $limit || $it === '') { $total += strlen($it); }\n\
               \x20   elseif ($i === -5) { $total -= 1; }\n\
               \x20 }\n\
               \x20 switch ($total) { case 0: break; default: break; }\n\
               \x20 return $total;\n}";
    // A: total, i, it, +=, -= = 5 ; B: strlen call, *= none -> calls only = 1
    // C: foreach, if, >, ||, ===, elseif, ===, case, default = 9
    assert_eq!(scores(src), vec![("branchy".to_string(), 5, 2, 9)]);
}

#[test]
fn match_arms_catch_and_ternary_count() {
    // C: match cond arm, its comparison, match default, catch, ternary
    let src = "<?php\nfunction messy(array $p): int {\n\
               \x20 try { $r = 1; } catch (\\Throwable $e) { $r = 2; }\n\
               \x20 $t = $r > 0 ? 1 : 0;\n\
               \x20 return match(true) { $t > 0 => 1, default => 0 } + $t;\n}";
    // A: r x2, t ; B: + ; C: catch, ternary, cmp, match arm, cmp-in-arm,
    // match default = 6
    assert_eq!(scores(src), vec![("messy".to_string(), 3, 1, 6)]);
}

#[test]
fn anonymous_functions_roll_into_enclosing_unit() {
    let src = "<?php\nfunction roller(array $fs): array {\n\
               \x20 $g = fn($v) => $v * 2;\n\
               \x20 $h = function ($v) use ($g) { return $g($v) + 1; };\n\
               \x20 return array_map($h, $fs);\n}";
    // A: g, h ; B: *, +(in closure), g($v) call, array_map call => 4
    assert_eq!(scores(src), vec![("roller".to_string(), 2, 4, 0)]);
}

#[test]
fn member_targets_bind_nothing() {
    let src = "<?php\nclass K {\n  private int $c = 0;\n  public function m(): void {\n\
               \x20 $this->c = 1;\n  }\n}";
    assert_eq!(dead(src), Vec::<String>::new());
}

#[test]
fn used_once_flags_single_pure_straightline_binding() {
    assert_eq!(
        used("<?php\nfunction ok(int $f): int {\n  $x = 42;\n  return $x + $f;\n}"),
        vec!["x"]
    );
}

#[test]
fn used_once_rejections() {
    let src = "<?php\nfunction rej(bool $f): int {\n\
               \x20 $a = id(1);\n\
               \x20 $b = 1; $b = 2;\n\
               \x20 $c = 1; $c += 1;\n\
               \x20 if ($f) { $e = 1; }\n\
               \x20 return (int) $a;\n}";
    assert_eq!(used(src), Vec::<String>::new());
}

#[test]
fn never_used_reports_dead_writes_once() {
    let src = "<?php\nfunction dd(): int {\n\
               \x20 $unused = 1;\n\
               \x20 [$q, $r] = [2, 3];\n\
               \x20 return $r;\n}";
    assert_eq!(dead(src), vec!["q", "unused"]);
}

#[test]
fn protocol_bindings_are_exempt() {
    let src = "<?php\nfunction proto(array $items): void {\n\
               \x20 foreach ($items as $k => $v) { echo $v; }\n\
               \x20 try { throw new \\RuntimeException('x'); }\n\
               \x20 catch (\\RuntimeException $e) { }\n\
               \x20 $_ignored = 9;\n}";
    assert_eq!(dead(src), vec!["e"]);
}

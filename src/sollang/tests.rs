//! End-to-end assertions over the Solidity backend: AbcSize vectors,
//! UsedOnce candidacy and NeverUsed reporting.

use super::{build, never_used_offenses, used_once_offenses};
use crate::abc::parse_vector;
use crate::paths::{Lang, parse_file_lang};

fn parse(src: &'static str) -> crate::sollang::SolFile<'static> {
    let tree = parse_file_lang(src.as_bytes(), Lang::Solidity).expect("solidity parses");
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
    let fm = parse(src);
    let mut v: Vec<_> = used_once_offenses(&fm)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

fn dead(src: &'static str) -> Vec<String> {
    let fm = parse(src);
    let mut v: Vec<_> = never_used_offenses(&fm)
        .into_iter()
        .map(|o| o.name)
        .collect();
    v.sort();
    v
}

#[test]
fn assignment_and_binary_count_as_a_and_b() {
    assert_eq!(
        scores(
            "contract T {\n  function simple(uint256 a) public pure returns (uint256) {\n\
             \x20   uint256 x = a + 1;\n\
             \x20   return x;\n  }\n}"
        ),
        vec![("simple".to_string(), 1, 1, 0)]
    );
}

#[test]
fn branches_loops_and_updates_tally() {
    // A: total, i head, += , -= , i++ = 5
    // B: none (member reads are free)
    // C: for, <, if, >, &&, !=, elseif-if, ==, while, > = 10
    let src = "contract T {\n  function branchy(uint256[] calldata items, uint256 limit)\n\
               \x20   external view returns (uint256) {\n\
               \x20   uint256 total = 0;\n\
               \x20   for (uint256 i = 0; i < items.length; i++) {\n\
               \x20     if (i > limit && items[i] != 0) { total += items[i]; }\n\
               \x20     else if (i == 7) { break; }\n\
               \x20   }\n\
               \x20   while (total > 100) { total -= 10; }\n\
               \x20   return total;\n  }\n}";
    assert_eq!(scores(src), vec![("branchy".to_string(), 5, 0, 10)]);
}

#[test]
fn state_writes_bind_nothing() {
    let src = "contract T {\n  mapping(address => uint256) balances;\n\n\
               \x20 function set(uint256 v) public {\n\
               \x20   balances[msg.sender] = v;\n\
               \x20 }\n}";
    assert_eq!(dead(src), Vec::<String>::new());
}

#[test]
fn constructors_are_units() {
    let got = scores("contract T {\n  constructor() {\n    uint256 x = 1 + 2;\n    x;\n  }\n}");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], ("T".to_string(), 1, 1, 0));
}

#[test]
fn used_once_flags_single_pure_straightline_binding() {
    assert_eq!(
        used(
            "contract T {\n  function ok(uint256 f) public pure returns (uint256) {\n\
              \x20   uint256 x = 42;\n\
              \x20   return x + f;\n  }\n}"
        ),
        vec!["x"]
    );
}

#[test]
fn used_once_rejections() {
    let src = "contract T {\n  function rej(bool f) external {\n\
               \x20   uint256 a = id();\n\
               \x20   uint256 b = 1; b = 2;\n\
               \x20   uint256 c = 1; c += 1;\n\
               \x20   if (f) { uint256 e = 1; }\n\
               \x20   emit Logged(a);\n  }\n  event Logged(uint256);\n}";
    assert_eq!(used(src), Vec::<String>::new());
}

#[test]
fn never_used_reports_dead_writes_once() {
    let src = "contract T {\n  function dd() public pure returns (uint256) {\n\
               \x20   uint256 unused = 1;\n\
               \x20   (uint256 q, uint256 r) = (2, 3);\n\
               \x20   return r;\n  }\n}";
    assert_eq!(dead(src), vec!["q", "unused"]);
}

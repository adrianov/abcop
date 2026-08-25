# abcop

Fast, multi-language static-analysis linter built around the **ABC software
metric** and a **used-once variable** detector. One parse per file, one walk
per metric, file-level parallelism — no language runtime required.

```text
lib/sinatra/base.rb:1254:0: C: Metrics/AbcSize: Assignment Branch Condition size for `error_block!` is too high. [<7, 14, 9> 18.06/17]
lib/foo.rb:12:2: W: UsedOnce: variable `tmp` is assigned once and read once -- consider inlining
src/main.rs: W: ModuleSize: 227 lines (>= 200) -- extract a coherent subunit
```

## Why

Two questions come up constantly during code review:

1. *How big is this function, really?* Line counts lie. The ABC metric counts
   what actually does work: assignments, branches and conditions.
2. *Can this variable just be inlined?* A local assigned once and read once is
   an indirection with no payoff — but inlining must not change behavior.

abcop answers both for whole trees in milliseconds, and a third rule keeps
source modules from growing without bound.

## Inspiration

abcop is inspired by [RuboCop](https://github.com/rubocop/rubocop) and by
[lizard](https://github.com/terryyin/lizard) — but mostly by the **ABC metric
itself**.

The ABC metric was devised by **Jerry Fitzpatrick** and introduced in his
article *"Applying the ABC Metric to C, C++, and Java"*, **C++ Report, June
1997**. He proposed it to overcome the drawbacks of counting lines of code:
ABC is strictly a *size* metric — it tallies assignments (A), branches (B:
explicit forward transfers such as calls), and conditions (C) — and combines
them into a single score, `sqrt(A² + B² + C²)`. RuboCop's `Metrics/AbcSize`
and lizard's family of size checks are both downstream of that idea.

From RuboCop we take the exact default counting rules for Ruby (verified
byte-for-byte against RuboCop 1.89's calculator) and its suppression-comment
workflow. From lizard we take the shape: one small tool that speaks many
languages. Everything else — the speed target, the used-once analysis, the
module-size budget — exists because those two tools stop where we wanted to
start.

## Advantages

- **Fast**: ~850k–940k lines/second per core batch on Apple M1 Pro. Scanning
  RuboCop's own source tree (943 files, 110k LOC) takes ~0.13 s for both
  metrics — roughly **39× faster** than `rubocop --only Metrics/AbcSize`
  (caching disabled), ~140× faster than a full default rubocop run.
- **RuboCop-compatible AbcSize**: identical formula, vector notation,
  threshold semantics and suppression comments for Ruby. Counting was verified
  against RuboCop 1.89's own calculator on real-world corpora
  (`scripts/compare_parity.py` joins per-method vectors; see Development).
  `# rubocop:disable` / `enable` / `todo` directives, including trailing form,
  block ranges, bare `Metrics` namespace; `rubocop:disable-next` is ignored,
  exactly like rubocop itself.
- **UsedOnce**: inline candidates only — pure right-hand side, unconditional
  write, straight-line dominance, single read outside macro input tokens.
  Parameters, pattern bindings, reassignments and shadowed names never qualify.
- **ModuleSize**: warns when a production module reaches 200 lines. Test
  suites, fixtures, lockfiles and schema dumps are exempt; Rust `#[cfg(test)]`
  tails don't count toward the budget.
- **No runtime hassle**: one static binary; parses via tree-sitter grammars,
  so machines without Ruby/Rust/Python installed can lint their code.
- **Deterministic**: identical output across runs; JSON mode for CI.

## Usage

Build (requires a Rust toolchain):

```sh
cargo build --release
./target/release/abcop [OPTIONS] PATH...
```

| Option | Default | Meaning |
|---|---|---|
| `[PATH]...` | — | files or directories (walked gitignore-aware) |
| `--format text\|json` | `text` | human-readable or machine-readable output |
| `--max-abc N` | `17` | report functions scoring above N |
| `--only abc\|used-once\|never-used` | all | run a single check |
| `--dump-tree FILE` | — | debug: print the syntax tree of one file |

Exit codes: `0` clean, `1` diagnostics reported, `2` usage error.

Examples:

```sh
abcop app lib                          # scan two trees, text output
abcop --format json lib > abcop.json   # JSON for CI dashboards
abcop --only used-once src             # inline-candidate hunt
abcop --max-abc 12 --only abc lib      # stricter ABC budget
```

JSON diagnostics carry `file`, `line`, `column`, `severity`, `rule`,
`message`, plus `score` and `vector` for ABC entries:

```json
{"rule":"Metrics/AbcSize","score":10.0,"vector":"<6, 8, 0>"}
```

### Checks

| Rule | Severity | Meaning |
|---|---|---|
| `Metrics/AbcSize` | C | function ABC score exceeds `--max-abc` |
| `UsedOnce` | W | local assigned once, read once, safe to inline |
| `NeverUsed` | W | local assigned but never read (dead writes) |
| `ModuleSize` | W | production module ≥ 200 lines |

### Directives

For Ruby sources (and, text-based, for Rust comments too) abcop honours
RuboCop suppression comments — trailing and block forms, cop lists or bare:

```ruby
def legacy_path # rubocop:disable Metrics/AbcSize
  ...
end
```

`rubocop:disable-next …` is editor-style and intentionally ignored, matching
rubocop itself.

## Supported languages

| Language | Files | Status |
|---|---|---|
| Ruby | `.rb .rake .ru .gemspec`, `Gemfile`, `Rakefile`, … | AbcSize parity + directives + UsedOnce |
| Rust | `.rs` | AbcSize + UsedOnce (spec in `src/rustlang.rs`) |
| JavaScript, TypeScript, C/C++, Objective-C, Swift | — | planned on the same engine |

## Benchmarks

Apple M1 Pro, warm cache, wall time:

| Corpus | Size | abcop (both rules) | rubocop `--only AbcSize`¹ | Full rubocop |
|---|---|---|---|---|
| rubocop/lib | 943 files, 110k LOC | **0.13 s** | 5.1 s (**~39× slower**) | 18.3 s |
| cargo registry sample | 6,603 files, 2.6M LOC (Rust) | **~2.8 s** (~940k LOC/s) | — | — |

¹ Compare like with like: run rubocop with `--cache false`; its result cache
makes repeat runs look near-instant otherwise.

## Development

```sh
cargo test          # unit tests: metric vectors, scope model, used-once gates
cargo clippy        # keep zero warnings
abcop src           # dogfood: abcop lints itself
```

`scripts/compare_parity.py` joins abcop JSON against
`rubocop --format json` output to verify per-method ABC vectors.

## Copyright

Copyright © 2026 Peter Adrianov. All rights reserved.

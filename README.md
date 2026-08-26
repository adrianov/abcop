# abcop

**Blazing-fast, opinionated, multi-language static-analysis linter**, written
in Rust with speed as a design constraint — not an afterthought. One parse
per file, one walk per metric, file-level parallelism, zero language runtime.
Whole trees in milliseconds; every run prints how many files it analysed and
how long it took.

```text
lib/sinatra/base.rb:1254:0: C: Metrics/AbcSize: Assignment Branch Condition size for `error_block!` is too high. [<7, 14, 9> 18.06/17]
lib/foo.rb:12:2: W: UsedOnce: variable `tmp` is assigned once and read once -- consider inlining
src/main.rs: W: ModuleSize: 227 lines (>= 200) -- extract a coherent subunit
132 files analysed in 0.09s, 7 abc offenses, 52 used-once offenses, 0 never-used warnings, 14 module-size warnings
```

## Why

Two questions come up constantly during code review:

1. *How big is this function, really?* Line counts lie. The ABC metric counts
   what actually does work: assignments, branches and conditions.
2. *Can this variable just be inlined?* A local assigned once and read once is
   an indirection with no payoff — but inlining must not change behavior.

abcop answers both for whole trees in milliseconds, and a third rule keeps
source modules from growing without bound.

## Built for CI in LLM-driven development

abcop exists to gate AI-written code in CI pipelines. That mission dictates
the rules it ships:

- **Short modules, low complexity — enforced, not wished for.** A 200-line
  module budget and per-function ABC limits keep every file small enough to
  hold in a human's head and inside an LLM's context window at once. Small,
  simple units are what models modify most reliably and what reviewers can
  actually read; when something breaks, the debugging surface is tiny by
  construction.
- **Fast enough to run on every push.** Sub-second whole-tree scans make
  linting free at the exact moment code is written — including code written
  by an agent that will never re-read it tomorrow.

## Opinionated by design

abcop takes sides and does not apologize for them:

- **No formatting or style rules — deliberately.** Formatting carries no
  signal about defects; it cannot catch a single bug, only generate churn,
  bikeshedding and diff noise. Formatting belongs to formatters, style to
  taste. abcop spends its budget exclusively on findings that change what you
  do next: a function too big to review, a dead write, an inline candidate.
- **Vendored, generated and test material is skipped by default.** Your CI
  minutes and your attention belong to production code. Name such a path
  explicitly when you genuinely want it reviewed.
- **One threshold to argue about (`--max-abc`), not fifty knobs.**

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
| `[PATH]...` | current MR | files or directories to analyse; **omitted, abcop scans your current merge request** (see `--mr`), falling back to the full tree outside a repository. Given: exactly those targets. All walking modes prune test/fixture trees (`spec/`, `tests/`, `fixtures/`, `testdata/`, …), vendored/build trees (`vendor/`, `node_modules/`, `target/`, `dist/`, `third_party/`, `coverage/`, `.terraform/`, `DerivedData/`, …), Rails `db/migrate/`, framework route tables (`config/routes.rb`, `config/routes/*.rb`) and generated files (`*.min.js`, `*.bundle.js`, protobuf `*_pb.rb` / `*_pb2.py` / `*.pb.go`) — name such a path explicitly to scan it; `--mr`/`--changed` scopes also drop framework route tables. Bare `abcop` covers the union of uncommitted work vs HEAD and the branch's changes vs its base |
| `--max-abc N` | `17` | report functions scoring above N |
| `--only abc\|used-once\|never-used` | all | run a single check |
| `--changed [--base REF]` | off | scan only git-changed files/functions vs REF (HEAD); hunks widened with `-W`, so a whole touched function counts as changed |
| `--full` | off | scan the whole production tree instead of the current MR (default skips stay active); bare `--full` targets the current directory |
| `--everything` | off | scan literally everything below the target: no gitignore, no hidden-file skipping, no vendored/generated/test pruning |
| `--dump-tree FILE` | — | debug: print the syntax tree of one file |

Exit codes: `0` clean, `1` diagnostics reported, `2` usage error.

Files are reported breadth-first (shallowest first) and ordered by
extension, then name inside each directory — so your quickest-to-review
top-level files come before deeply nested ones. The parallel walk is
unordered internally; the deterministic order comes from a multi-key sort
over recorded depths.

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

### Changed-code workflow

Review what you just wrote instead of a decade of legacy:

```sh
git checkout -b feature/x     # branch off main
# ... work ...
abcop --mr --only abc src     # only functions you touched on this branch
```

Committing straight to main? The same command switches to a 36-hour window
automatically (`<default-branch>@{36.hours.ago}`) — enough to cover work
resumed from the previous morning. Force an explicit base with
`--base <ref>`. `--changed` remains available for plain working-tree diffs vs
any ref.

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

## Caching

Repeated scans of unchanged trees are served from a content-addressed
user-wide cache (`$XDG_CACHE_HOME/abcop`, falling back to `~/.cache/abcop`),
making editor-loop and pre-commit invocations ~5× faster:

```sh
abcop lib          # cold: full analysis
abcop lib          # warm: unchanged files load from cache (~5x faster)
abcop --no-cache   # force full analysis
```

Cache keys include file contents, tool version, rule revision, threshold,
selected checks and path — stale results are impossible, and entries from
different projects never collide. Entries live in a single embedded
key-value database file (`cache.redb`), auto-pruned to the 20 000 most
recent entries; disable entirely with `--no-cache`, relocate with
`ABCOP_CACHE_DIR=/path`. Nothing is ever written inside your project.

## Supported languages

| Language | Files | Status |
|---|---|---|
| Ruby | `.rb .rake .ru .gemspec`, `Gemfile`, `Rakefile`, … | AbcSize parity + directives + UsedOnce |
| Rust | `.rs` | AbcSize + UsedOnce (spec in `src/rustlang.rs`) |
| JavaScript | `.js .mjs .cjs .jsx` | AbcSize + directives |
| TypeScript | `.ts .tsx .mts .cts` | AbcSize + directives |
| C / C++ | `.c .h .cc .cpp .cxx .hpp .hxx .hh` | AbcSize + directives |
| Objective-C | `.m .mm` | AbcSize + directives |
| Swift | `.swift` | AbcSize + directives |
| Python | `.py .pyi .pyw` | AbcSize + UsedOnce + NeverUsed (spec in `src/pylang/`) |

C-family scoring lives in `src/clike.rs`: named declarations are units,
anonymous function-likes roll into their enclosing unit, and nested units
never double-count. UsedOnce/NeverUsed are additionally implemented for
Python (`src/pylang/`); elsewhere they remain Ruby/Rust-only by design --
their safety proofs are language-specific.

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

## License

Released under the **GNU General Public License v3 or later**
(SPDX: `GPL-3.0-or-later`). See [LICENSE](LICENSE).

Copyright © 2026 Peter Adrianov. All rights reserved.

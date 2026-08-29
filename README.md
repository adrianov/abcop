# abcop

**One self-contained binary that lints every language in your stack** —
Ruby, Rust, Python, Go, PHP, Java, C#, Dart, JavaScript, TypeScript, C, C++,
Objective-C, Swift and Solidity — with no runtimes, no plugins, no
per-language installs. Written in
Rust with speed as a design constraint: one parse per file, one walk per
metric, file-level parallelism, all grammars compiled in. Whole trees in
milliseconds; every run prints how many files it analysed and how long it
took.

```text
lib/sinatra/base.rb:1254:0: C: Metrics/AbcSize: Assignment Branch Condition size for `error_block!` is too high. [<7, 14, 9> 18.06/17]
lib/foo.rb:12:2: W: UsedOnce: variable `tmp` is assigned once and read once -- consider inlining
src/main.rs: W: Metrics/ModuleAbcSize: Assignment Branch Condition size for module is too high. [<80, 200, 60> 228.04/90] -- extract a coherent subunit
132 files analysed in 0.09s, 7 abc offenses, 52 used-once offenses, 0 never-used warnings, 14 module-abc warnings
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

- **Short modules, low complexity — enforced, not wished for.** A module ABC
  budget (≈ a typical 200-line Ruby file) and per-function ABC limits keep
  every file small enough to hold in a human's head and inside an LLM's
  context window at once. Small, simple units are what models modify most
  reliably and what reviewers can actually read; when something breaks, the
  debugging surface is tiny by construction.
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
module ABC budget — exists because those two tools stop where we wanted to
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
- **ModuleAbcSize**: warns when a production module's summed method ABC
  exceeds 90 (Fitzpatrick total: sum `<A,B,C>` then one magnitude — calibrated
  to a typical ~200-line Ruby file). Test suites, fixtures, lockfiles and
  schema dumps are exempt; Rust `#[cfg(test)]` tails don't count toward the
  budget.
- **One tool, zero dependencies.** A single self-contained binary replaces
  the per-language linter fleet — no Ruby gems, no pip/npm packages, no
  plugins, no version drift between machines. Every grammar is compiled
  in, so it lints code on machines without Ruby/Rust/Python/Go toolchains
  installed: CI images, containers, clean checkouts, a colleague's laptop.
  The result cache is an embedded pure-Rust database file — no external
  services to run or configure.
- **Deterministic**: identical output across runs; JSON mode for CI.

## Usage

Build (requires a Rust toolchain):

```sh
cargo build --release
./target/release/abcop [OPTIONS] PATH...
```

| Option | Default | Meaning |
|---|---|---|
| `[PATH]...` | auto | files or directories to analyse; omitted → auto-selected scope, see [Scope selection](#scope-selection) below |
| `--max-abc N` | `17` | report functions scoring above N |
| `--only abc\|used-once\|never-used` | all | run a single check |
| `--full` | off | scan the whole production tree instead of the scoped run (default skips stay active); bare `--full` targets the current directory |
| `--everything` | off | scan literally everything below the target: no gitignore, no hidden-file skipping, no vendored/generated/test pruning |
| `--format text\|json` | `text` | machine-readable JSON for CI dashboards |
| `--mr` | off | select the current-MR scope explicitly (uncommitted plus branch commits vs base); the bare default picks uncommitted-only when the tree is dirty |
| `--uncommitted` | off | scan only uncommitted work: working-tree and index edits vs `HEAD` plus untracked files — no branch/base diff; requires a repository |
| `--no-cache` | off | skip the on-disk result cache for this run |
| `--dump-tree FILE` | — | debug: print the syntax tree of a single file

### Scope selection

**Explicit paths** — exactly those targets are analysed.

**Omitted** — abcop auto-selects the narrowest scope still reviewing real work, in order:

1. uncommitted work vs `HEAD`, when the tree is dirty
2. your current merge request (see `--mr`)
3. the full tree, outside a repository

The choice is announced on stderr — a silently narrowed scope looks
identical to a requested one, which is exactly what misleads — and
`--mr`/`--full` pin the wider scopes explicitly.

**Every walking mode prunes:**

- test/fixture trees — `spec/`, `tests/`, `fixtures/`, `testdata/`, …
- vendored/build trees — `vendor/`, `node_modules/`, `target/`, `dist/`, `third_party/`, `coverage/`, `.terraform/`, `DerivedData/`, …
- Rails `db/migrate/`
- framework route tables — `config/routes.rb`, `config/routes/*.rb`
- generated files — `*.min.js`, `*.bundle.js`, protobuf `*_pb.rb` / `*_pb2.py` / `*.pb.go`

Name such a path explicitly to scan it anyway. A scoped run reviews only
its changed files, and third-party/vendored material in a diff is never
pulled into review surface, no matter which path opts it in.

**Route tables (`config/routes.rb`, `config/routes/*.rb`) are never review
surface.** They are declarative wiring: nearly every line added is an
endpoint someone asked for, so AbcSize/ModuleAbcSize findings there are noise
with no action.

**Third-party material is never scoped review surface.** A diff that
touches `vendor/`, `node_modules/`, `db/migrate/` or a generated file does
not make it owned production code — size and complexity findings there
carry no action you can take upstream. Name the path explicitly
(`abcop vendor/foo.c`) when you genuinely forked and own it.

**In scoped runs (bare `abcop` or explicit `--mr`), ModuleAbcSize fires only
when your diff touched ≥100 lines of that module** (untracked files count
as fully changed) — and this applies to **any** module, spec/test files
included: a hundred changed lines in a spec means the extraction
conversation is on the table there too. Rationale: scoped reviews exist to
keep changes compact — easier to review and less likely to drift out of
the MR's task scope. A three-line patch into a 500-line legacy module
should not gate your review for a size problem you did not cause;
refactor-scale diffs are exactly where extracting a coherent subunit is
expected. Full scans (`--full`, `--everything`) keep reporting every
oversized module.

**Code rules run in tests; only ModuleAbcSize exempts them by default.**
AbcSize, UsedOnce and NeverUsed stay active in `spec/`, `test/` and
friends: tests are code, dead bindings and inline candidates smell just as
much there. ModuleAbcSize's test-tree exemption lifts automatically once a
scoped diff crosses the 100-line threshold above.

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
`message`, plus `score` and `vector` for ABC entries (method and module):

```json
{"rule":"Metrics/AbcSize","score":10.0,"vector":"<6, 8, 0>"}
{"rule":"Metrics/ModuleAbcSize","score":120.5,"vector":"<40, 100, 40>"}
```

### Checks

| Rule | Severity | Meaning |
|---|---|---|
| `Metrics/AbcSize` | C | function ABC score exceeds `--max-abc` |
| `UsedOnce` | W | local assigned once, read once, safe to inline |
| `NeverUsed` | W | local assigned but never read (dead writes) |
| `Metrics/ModuleAbcSize` | W | module ABC (summed method vectors) exceeds 90 |

### Changed-code workflow

Review what you just wrote instead of a decade of legacy:

```sh
git checkout -b feature/x     # branch off main
# ... work ...
abcop --mr --only abc src     # only functions you touched on this branch
abcop --uncommitted          # pre-commit check: just your working-tree edits
abcop                        # bare: auto — uncommitted work when dirty, branch diff otherwise
```

Committing straight to main? The MR scope switches to a 36-hour window
automatically — enough to cover work resumed from the previous morning.
A dirty tree skips that machinery entirely: the bare default narrows to
just the working-tree edits (plus untracked files) and says so on
stderr; `--mr` forces the full branch union when you want it. Explicitly
narrowed runs (`--uncommitted`) fail loudly outside a repository rather
than silently widening to the full tree.

### Directives

In every supported language except Rust, abcop honours RuboCop-style
suppression comments (`#` or `//`) — trailing and block forms, cop lists
or bare:

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

One binary, fifteen languages, nothing to install per language — one
output format and one CI gate.

| Language | Files | Checks |
|---|---|---|
| Ruby | `.rb .rake .ru .gemspec`, `Gemfile`, `Rakefile`, … | all four, RuboCop-parity counting |
| Rust | `.rs` | all four |
| Python | `.py .pyi .pyw` | all four |
| Go | `.go` | all four |
| PHP | `.php` | all four |
| Java | `.java` | all four |
| C# | `.cs` | all four |
| Solidity | `.sol` | all four |
| JavaScript | `.js .mjs .cjs .jsx` | all four |
| TypeScript | `.ts .tsx .mts .cts` | all four |
| C / C++ | `.c .h .cc .cpp .cxx .hpp .hxx .hh` | all four |
| Objective-C | `.m .mm` | all four |
| Swift | `.swift` | all four |
| Dart | `.dart` | all four |

Every row runs the same scope-model engine — static spec tables describe
each grammar (which kinds bind, read, open scopes); one dispatcher
evaluates UsedOnce, NeverUsed, purity-gated inlining and ModuleAbcSize on
top. C/C++/Objective-C conventions: file-scope globals are out of reach
of single-file analysis and never reported; loop-head variables are
protocol; writing a struct field also reads the object itself.

Scoring semantics stay uniform across languages: named declarations are
the measured units, anonymous function-likes roll into their enclosing
unit, and nested units never double-count.

## Benchmarks

Apple M1 Pro, warm cache, wall time:

| Corpus | Size | abcop (all checks) | rubocop `--only AbcSize`¹ | Full rubocop |
|---|---|---|---|---|
| rubocop/lib | 943 files, 110k LOC | **0.13 s** | 5.1 s (**~39× slower**) | 18.3 s |
| cargo registry sample | 6,603 files, 2.6M LOC (Rust) | **~2.8 s** (~940k LOC/s) | — | — |

¹ Compare like with like: run rubocop with `--cache false`; its result cache
makes repeat runs look near-instant otherwise.

## Development

```sh
cargo test                # unit tests: metric vectors, scope model, used-once gates
cargo build --all-targets # keep zero warnings (including test targets)
abcop src                 # dogfood: abcop lints itself
```

Layout of the shared scope-model engine:

- `src/scope_model/backend.rs` — the `Backend` contract: three accessors
  plus default bindings every collector inherits (`bind_var`,
  `bind_declarator_with_rhs_field`, `walk_children`, `rebind_local`).
- `src/scope_model/walk.rs` — the `Spec` tables and the dispatcher that
  consumes everything needing no language-specific judgment.
- `src/clike/scope.rs` + `swift.rs` — JS/TS and Swift collectors; each is
  a `Spec` static table plus custom arms for genuinely language-specific
  node kinds.
- `src/clike/purity.rs` — shared RHS-purity predicates gating inline
  candidates; `src/sollang/decl.rs` — Solidity declaration/tuple-head
  binding. Evaluation lives in `scope_model::eval`, independent of any
  grammar.

Per-language behavioral vectors live beside their engine: AbcSize score
vectors in `clike/tests_abc.rs`, UsedOnce/NeverUsed end-to-end cases in
`clike/tests.rs` and `sollang/tests.rs`. Grammar-node kinds are probed
with `abcop --dump-tree FILE` — never trusted from grammar docs.

`scripts/compare_parity.py` joins abcop JSON against
`rubocop --format json` output to verify per-method ABC vectors.

## License

Released under the **GNU General Public License v3 or later**
(SPDX: `GPL-3.0-or-later`). See [LICENSE](LICENSE).

Copyright © 2026 Peter Adrianov. All rights reserved.

# abcop

**A must-have gate for AI-written code.** Agents ship faster than humans
can re-read; abcop keeps that code *understandable* by gating function and
module ABC complexity so every unit fits a human head and an LLM context
window. Diagnostics are also a **hook for automated refactoring** —
extract a method, split a module — so CI rejects bad growth *and* points
agents at concrete maintainability fixes.

One self-contained binary across Ruby, Rust, Python, Go, PHP, Java, C#,
Dart, JavaScript, TypeScript, C, C++, Objective-C, Swift, Solidity, Zig
and Haskell —
no runtimes, no plugins, no per-language installs. Written in Rust for
speed: one parse per file, one walk per metric, grammars compiled in;
whole trees in milliseconds.

## Install

```sh
brew install adrianov/abcop/abcop
```

```sh
cargo install abcop
```

macOS (`.tar.gz` from [GitHub Releases](https://github.com/adrianov/abcop/releases); binary + man):

```sh
# Apple Silicon — use the *-x86_64-apple-darwin.tar.gz asset on Intel Macs
curl -LO https://github.com/adrianov/abcop/releases/download/v0.18.0/abcop-0.18.0-aarch64-apple-darwin.tar.gz
tar -xzf abcop-0.18.0-aarch64-apple-darwin.tar.gz
sudo cp abcop-0.18.0-aarch64-apple-darwin/abcop /usr/local/bin/
sudo mkdir -p /usr/local/share/man/man1
sudo cp abcop-0.18.0-aarch64-apple-darwin/abcop.1 /usr/local/share/man/man1/
```

Ubuntu / Debian (`.deb` from [GitHub Releases](https://github.com/adrianov/abcop/releases); amd64, Ubuntu 22.04+ / Debian bookworm+):

```sh
# example for v0.18.0 — use the asset name from the release page
curl -LO https://github.com/adrianov/abcop/releases/download/v0.18.0/abcop_0.18.0-1_amd64.deb
sudo dpkg -i abcop_0.18.0-1_amd64.deb
man abcop
```

## Example run

```text
lib/sinatra/base.rb:1254:0: C: Metrics/AbcSize: Assignment Branch Condition size for `error_block!` is too high. [<7, 14, 9> 18.06/17]
src/main.rs: W: Metrics/ModuleAbcSize: Assignment Branch Condition size for module is too high. [<80, 200, 60> 228.04/120] -- extract a coherent subunit
132 files analysed in 0.09s, 7 abc offenses, 0 used-once offenses, 0 never-used warnings, 14 module-abc warnings
```

## Why ABC, not line counts

Line counts lie. The [ABC metric](https://en.wikipedia.org/wiki/ABC_Software_Metric)
(Jerry Fitzpatrick, *C++ Report*, June 1997) counts what does work —
assignments (A), branches (B), conditions (C) — as `sqrt(A² + B² + C²)`.
Fitzpatrick defined both method and module scope; abcop gates both:

| Rule | Severity | Meaning |
|---|---|---|
| `Metrics/AbcSize` | C | function ABC above `--max-abc` (default 17) |
| `Metrics/ModuleAbcSize` | W | module ABC above `--max-module-abc` (default 120) |
| `UsedOnce` | W | local written once, read once — consider inlining |
| `NeverUsed` | W | local written, never read |

A sparse wrapper and a dense god-object can share a line count; ABC
separates them. Complexity is the gate — never a line budget. UsedOnce /
NeverUsed are secondary.

## Built for AI workflows

- **Understandable by construction.** AbcSize 17 and ModuleAbcSize 120 keep
  every unit inside a human head and an LLM context window. Models edit
  small units reliably; when something breaks, the blast radius is tiny.
- **Refactoring hook.** Exit code `1` plus stable JSON/JSONL diagnostics
  (`rule`, `score`, `vector`, `message`) feed agents and scripts: split
  oversized modules, extract hot methods. Each finding is an actionable
  maintainability step, not a style nit.
- **Fast enough for every push.** Sub-second whole-tree scans — including
  code an agent will never re-read tomorrow.
- **Signal only.** No formatting or style cops. Vendored, generated and
  test trees are skipped by default. Two knobs: `--max-abc` and
  `--max-module-abc`.

Inspired by [RuboCop](https://github.com/rubocop/rubocop) (Ruby AbcSize
parity and `# rubocop:disable` directives) and
[lizard](https://github.com/terryyin/lizard) (one tool, many languages).
Ruby's counting matches RuboCop 1.89 byte-for-byte
(`scripts/compare_parity.py`).

## Advantages

- **~850k–940k LOC/s** per core (Apple M1 Pro). RuboCop's lib tree
  (943 files, 110k LOC): **0.13 s** — ~39× faster than
  `rubocop --only Metrics/AbcSize` (cache off), ~140× a full rubocop run.
- **One binary, zero deps** — grammars and an embedded result cache
  (`cache.redb`) compiled in; works on clean CI images with no language
  toolchains.
- **Deterministic** — same findings every run; text / JSON / JSONL stream
  as files finish; `--sort-by-score` buffers for worst-first emit.

## Usage

```sh
abcop [OPTIONS] PATH...
```

| Option | Default | Meaning |
|---|---|---|
| `[PATH]...` | auto | targets; omitted → [scope selection](#scope-selection) |
| `--max-abc N` | `17` | function ABC ceiling |
| `--max-module-abc N` | `120` | module ABC ceiling |
| `--only abc\|used-once\|never-used` | all | single check |
| `--full` | off | whole production tree (default skips stay on) |
| `--everything` | off | no gitignore / hidden / vendored pruning |
| `--format text\|json\|jsonl` | `text` | CI-friendly output |
| `--sort-by-score` | off | highest ABC first |
| `--mr` | off | MR scope (uncommitted + branch vs base) |
| `--uncommitted` | off | working-tree + index + untracked vs `HEAD` only |
| `--no-cache` | off | skip on-disk cache |
| `--dump-tree FILE` | — | debug syntax tree |

Exit codes: `0` clean, `1` findings, `2` usage error.

```sh
abcop app lib                          # two trees
abcop --format jsonl lib > abcop.jsonl # streaming CI / agent input
abcop --only used-once src             # inline candidates
abcop --max-abc 12 --only abc lib      # stricter function budget
abcop --max-module-abc 80 lib           # stricter module budget
abcop --sort-by-score --only abc lib   # worst first
abcop --uncommitted                    # pre-commit / agent loop
abcop --mr --only abc                  # this branch's touched units
```

JSON diagnostics include `file`, `line`, `column`, `severity`, `rule`,
`message`, plus `score` / `vector` for ABC rules:

```json
{"rule":"Metrics/AbcSize","score":10.0,"vector":"<6, 8, 0>"}
{"rule":"Metrics/ModuleAbcSize","score":120.5,"vector":"<40, 100, 40>"}
```

### Scope selection

**Named paths** — those targets only.

**Omitted** — narrowest useful scope, announced on stderr:

1. uncommitted work vs `HEAD` if the tree is dirty
2. else current MR (`--mr` forces this)
3. else full tree (outside a repo)

Default walks prune test/fixture trees, vendored/build output
(`vendor/`, `node_modules/`, `target/`, …), `db/migrate/`, route tables
(`config/routes.rb`, `config/routes/*.rb`), and generated names
(`*.min.js`, `*_pb.go`, …). Name a path explicitly to scan it anyway.
Third-party, route-table, and fixture paths are never scoped review
surface — a diff through `vendor/` or `tests/fixtures/` does not make
that material owned code.

**Scoped ModuleAbcSize** re-sums only methods that intersect the diff and
compares that total to `--max-module-abc` (default 120; untracked =
every method). A small patch into an oversized legacy file stays quiet
unless the touched methods themselves exceed the ceiling; AbcSize still
reports any changed method over `--max-abc`. Full scans (`--full`,
`--everything`) report every production module over the ceiling;
ModuleAbcSize still exempts test trees on full scans (scoped runs can
flag them when changed methods sum over the limit). UsedOnce /
NeverUsed always follow the changed lines.

On a dirty tree the bare default is uncommitted-only; `--mr` takes the
full branch union. Commits straight to main use a 36-hour window when no
branch base applies. `--uncommitted` fails outside a repository instead
of silently widening.

### Directives

Everywhere except Rust, RuboCop-style `#` / `//` suppressions work
(trailing and block; bare `Metrics` allowed). `rubocop:disable-next` is
ignored, matching rubocop.

```ruby
def legacy_path # rubocop:disable Metrics/AbcSize
  ...
end
```

## Caching

Content-addressed cache under `$XDG_CACHE_HOME/abcop` (or `~/.cache/abcop`;
override with `ABCOP_CACHE_DIR`). Warm reruns ~5× faster. Keys cover
contents, version, rule revision, threshold, checks and path — no
cross-project collisions. Auto-pruned to 20 000 entries; `--no-cache`
disables. Nothing is written inside the project.

## Supported languages

Seventeen languages, four rules each, one CI gate.

| Language | Files | Notes |
|---|---|---|
| Ruby | `.rb .rake .ru .gemspec`, `Gemfile`, `Rakefile`, … | RuboCop-parity AbcSize |
| Rust | `.rs` | |
| Python | `.py .pyi .pyw` | |
| Go | `.go` | |
| PHP | `.php` | |
| Java | `.java` | |
| C# | `.cs` | |
| Solidity | `.sol` | |
| Dart | `.dart` | |
| Zig | `.zig` | |
| Haskell | `.hs .lhs` | |
| JavaScript | `.js .mjs .cjs .jsx` | |
| TypeScript | `.ts .tsx .mts .cts` | |
| C / C++ | `.c .h .cc .cpp .cxx .hpp .hxx .hh` | `.h` via C++ grammar |
| Objective-C | `.m .mm` | |
| Swift | `.swift` | |

Named declarations are measured units; anonymous function-likes roll into
the enclosing unit; nested units never double-count. C-family: file-scope
globals are out of single-file reach; loop-head locals are protocol;
field writes also read the object; export-macro class forms
(`class UTIL_EXPORT Foo`) are skipped for variable rules.

## Benchmarks

Apple M1 Pro, warm cache:

| Corpus | Size | abcop | rubocop `--only AbcSize`¹ | Full rubocop |
|---|---|---|---|---|
| rubocop/lib | 943 files, 110k LOC | **0.13 s** | 5.1 s (~39×) | 18.3 s |
| cargo registry sample | 6,603 files, 2.6M LOC | **~2.8 s** | — | — |

¹ `--cache false` — rubocop's own cache otherwise makes repeats look free.

## Development

```sh
cargo test
cargo build --all-targets   # zero warnings
abcop src                   # dogfood
```

Shared engine: `src/scope_model/` (`backend`, `walk`, `eval`); language
collectors sit beside each backend (`clike/`, `sollang/`, …). Probe
grammar nodes with `abcop --dump-tree FILE`. Parity:
`scripts/compare_parity.py`.

## License

**GNU GPL v3 or later** (SPDX: `GPL-3.0-or-later`). See [LICENSE](LICENSE).

Copyright © 2026 Peter Adrianov. All rights reserved.

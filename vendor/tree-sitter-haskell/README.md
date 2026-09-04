# tree-sitter-haskell

[![CI][ci]](https://github.com/tree-sitter/tree-sitter-haskell/actions/workflows/ci.yml)
[![discord][discord]](https://discord.gg/w7nTvsVJhm)
[![matrix][matrix]](https://matrix.to/#/#tree-sitter-chat:matrix.org)
[![crates][crates]](https://crates.io/crates/tree-sitter-haskell)
[![npm][npm]](https://www.npmjs.com/package/tree-sitter-haskell)
[![pypi][pypi]](https://pypi.org/project/tree-sitter-haskell)

Haskell grammar for [tree-sitter].

# References

- [Haskell 2010 Language Report – Syntax References][ref]
- [GHC Language Extensions][ext]

# Supported Language Extensions

Each extension is supported ✅, unsupported ❌, or marked ➖️ when it does not
affect parsing:

- AllowAmbiguousTypes ➖️
- ApplicativeDo ➖️
- Arrows ❌
- BangPatterns ✅
- BinaryLiterals ✅
- BlockArguments ✅
- CApiFFI ✅
- ConstrainedClassMethods ✅
- ConstraintKinds ✅
- CPP ✅
- CUSKs ✅
- DataKinds ✅
- DatatypeContexts ✅
- DefaultSignatures ✅
- DeriveAnyClass ➖️
- DeriveDataTypeable ➖️
- DeriveFoldable ➖️
- DeriveFunctor ➖️
- DeriveGeneric ➖️
- DeriveLift ➖️
- DeriveTraversable ➖️
- DerivingStrategies ✅
- DerivingVia ✅
- DisambiguateRecordFields ➖️
- DuplicateRecordFields ➖️
- EmptyCase ✅
- EmptyDataDecls ✅
- EmptyDataDeriving ✅
- ExistentialQuantification ✅
- ExplicitForAll ✅
- ExplicitNamespaces ✅
- ExtendedDefaultRules ➖️
- FlexibleContexts ✅
- FlexibleInstances ✅
- ForeignFunctionInterface ✅
- FunctionalDependencies ✅
- GADTs ✅
- GADTSyntax ✅
- GeneralisedNewtypeDeriving ➖️
- GHCForeignImportPrim ✅
- Haskell2010 ➖️
- Haskell98 ➖️
- HexFloatLiterals ✅
- ImplicitParams ✅
- ImplicitPrelude ➖️
- ImportQualifiedPost ✅
- ImpredicativeTypes ➖️
- IncoherentInstances ➖️
- InstanceSigs ✅
- InterruptibleFFI ✅
- KindSignatures ✅
- LambdaCase ✅
- LexicalNegation ❌
- LiberalTypeSynonyms ✅
- LinearTypes ✅
- ListTuplePuns ✅
- MagicHash ✅
- Modifiers ❌
- MonadComprehensions ➖️
- MonadFailDesugaring ➖️
- MonoLocalBinds ➖️
- MonomorphismRestriction ➖️
- MultiParamTypeClasses ✅
- MultiWayIf ✅
- NamedFieldPuns ✅
- NamedWildCards ✅
- NegativeLiterals ➖️
- NondecreasingIndentation ✅
- NPlusKPatterns ➖️
- NullaryTypeClasses ✅
- NumDecimals ➖️
- NumericUnderscores ✅
- OverlappingInstances ➖️
- OverloadedLabels ✅
- OverloadedLists ➖️
- OverloadedRecordDot ✅
- OverloadedRecordUpdate ✅
- OverloadedStrings ➖️
- PackageImports ✅
- ParallelListComp ✅
- PartialTypeSignatures ✅
- PatternGuards ✅
- PatternSynonyms ✅
- PolyKinds ➖️
- PostfixOperators ➖️
- QualifiedDo ✅
- QuantifiedConstraints ✅
- QuasiQuotes ✅
- Rank2Types ✅
- RankNTypes ✅
- RebindableSyntax ➖️
- RecordWildCards ➖️
- RecursiveDo ✅
- RequiredTypeArguments ✅
- RoleAnnotations ✅
- Safe ➖️
- ScopedTypeVariables ✅
- StandaloneDeriving ✅
- StandaloneKindSignatures ✅
- StarIsType ✅
- StaticPointers ❌
- Strict ➖️
- StrictData ✅
- TemplateHaskell ✅
- TemplateHaskellQuotes ✅
- TraditionalRecordSyntax ➖️
- TransformListComp ✅
- Trustworthy ➖️
- TupleSections ✅
- TypeAbstractions ✅
- TypeApplications ✅
- TypeData ✅
- TypeFamilies ✅
- TypeFamilyDependencies ✅
- TypeInType ✅
- TypeOperators ✅
- TypeSynonymInstances ➖️
- UnboxedSums ✅
- UnboxedTuples ✅
- UndecidableInstances ➖️
- UndecidableSuperClasses ➖️
- UnicodeSyntax ✅
- UnliftedFFITypes ➖️
- UnliftedNewtypes ✅
- Unsafe ➖️
- ViewPatterns ✅

# Bugs

## CPP

Preprocessor `#elif` and `#else` cannot be handled correctly: the parser would
need to restore the state from the matching `#if`. As a workaround, bodies in
those alternate branches are parsed as part of the directive itself.

# Querying

The grammar defines several
[supertypes](https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types)
that group related node kinds under one name.

Supertype names do not appear as extra nodes in parse trees, but queries can
use them in two ways:

- As an alias that matches any of their subtypes
- As a prefix before one subtype, matching that symbol only when it is produced
  by the supertype

For example, `(expression)` matches the nodes `infix`, `record`, `projection`,
`constructor`, and the second and third `variable` in this tree for
`cats <> Cat {mood = moods.sleepy}`:

```
(infix
  (variable)
  (operator)
  (record
    (constructor)
    (field_update
      (field_name (variable))
      (projection (variable) (field_name (variable)))))))))
```

The two `variable` nodes under `field_name` (`mood` and `sleepy`) are field
names inside a `record` expression, not expressions themselves.

To match only `variable` nodes that are expressions, use the prefixed form:
`(expression/variable)` matches `cats` and `moods` only.

Supertypes in this grammar:

- [`expression`](./grammar/exp.js)

  Constructs valid in expression position, excluding type applications,
  explicit types, and expression signatures.

- [`pattern`](./grammar/pat.js)

  Constructs valid in pattern position, excluding type binders, explicit types,
  and pattern signatures.

- [`type`](./grammar/type.js)

  Atomic types (unambiguous associativity: brackets, variables, type
  constructors), applied types, and infix types.

- [`quantified_type`](./grammar/type.js)

  Types introduced by `forall`, a context, or a function parameter.

- [`constraint`](./grammar/constraint.js)

  Much like `type`, but for use in contexts.

- [`constraints`](./grammar/constraints.js)

  The constraint-side counterpart of `quantified_type` (`forall` or context).

- [`type_param`](./grammar/type.js)

  Atomic nodes in type and class heads — for example the three nodes after `A`
  in `data A @k a (b :: k)`.

- [`declaration`](./grammar/module.js)

  Top-level declarations such as functions and data types.

- [`decl`](./grammar/decl.js)

  Declarations that are also valid in local bindings (`let` / `where`) and in
  class and instance bodies, except fixity declarations. Covers `signature`,
  `function`, and `bind`.

- [`class_decl` and `instance_decl`](./grammar/class.js)

  Declarations allowed in classes and instances, including associated type and
  data families.

- [`statement`](./grammar/exp.js)

  Forms of `do`-notation statements.

- [`qualifier`](./grammar/exp.js)

  Forms of list-comprehension qualifiers.

- [`guard`](./grammar/exp.js)

  Forms of guards on function equations and case alternatives.

# Development

The [tree-sitter CLI][cli] is the main tool for generating and testing this
grammar. Other parts of the project need the extra tools described below.

Some tools come from `npm` — for example `npx tree-sitter` runs the CLI. If
`tree-sitter` is not on your `PATH`, prefix the commands in the following
sections with `npx`.

## Output path

The CLI writes the parser shared library under `$TREE_SITTER_LIBDIR`. When that
variable is unset, it defaults to `$HOME/.cache/tree-sitter/lib`.

To keep development builds out of that global directory, point the variable at
a local path:

```
export TREE_SITTER_LIBDIR=$PWD/.lib
```

## The grammar

`grammar.js` is the entry point for the grammar’s production rules. See the
[tree-sitter documentation][grammar-docs] for the full syntax and semantics.

Parsing begins with the first entry in the `rules` field:

```javascript
{
  rules: {
    haskell: $ => seq(
      optional($.header),
      optional($._body),
    ),
  }
}
```

## Generating the parser

The first development step turns the JavaScript rule definitions into C in
`src/parser.c`:

```
$ tree-sitter generate
```

This also writes `src/grammar.json` and `src/node-types.json`.

## Compiling the parser

Most of the test tools below compile the C automatically. To generate and build
in one step:

```
$ tree-sitter generate --build
```

With `$TREE_SITTER_LIBDIR` set as above, the shared object lands at
`$PWD/.lib/haskell.so`.

Besides generated `src/parser.c`, tree-sitter also compiles and links
`src/scanner.c`. That file is the *external scanner*: a custom extension of the
built-in lexer for constructs that the JavaScript grammar cannot express
efficiently (notably Haskell layout).

### WebAssembly

The parser can also be built as WebAssembly (requires `emscripten`):

```
$ tree-sitter build --wasm
```

The binary is written to `$PWD/tree-sitter-haskell.wasm`.

## Testing the parser

The core tests are code snippets paired with reference ASTs in
`./test/corpus/*.txt`:

```
$ tree-sitter test
```

Run a single test by matching (a substring of) its description with `-f`:

```
$ tree-sitter test -f 'module: exports empty'
```

Further test suites:

- `test/parse/run.bash [update] [test names ...]` parses `test/parse/*.hs` and
  compares output to `test/parse/*.target`. With `update` as the first
  argument, it rewrites the `.target` for the first failing test.

- `test/query/run.bash [update] [test names ...]` parses `test/query/*.hs`,
  runs the queries in `test/query/*.query`, and compares output to
  `test/query/*.target` (same `update` behaviour as `test/parse`).

- `test/rust/parse-test.rs` uses tree-sitter’s Rust API to assert terminal
  ranges a bit more conveniently. Needs `cargo`; run with `cargo test` (also
  runs `bindings/rust` tests).

- `test/parse-libs [wasm]` clones a set of Haskell libraries into `test/libs`
  and parses each codebase. `test/parse-libs wasm` uses the WebAssembly parser.
  Requires `bc`.

- `test/parse-lib name [wasm]` parses only library `name` in that directory
  (no clone).

### Debugging

The shared library from `tree-sitter test` includes debug symbols. If the
scanner segfaults, inspect the backtrace with `coredumpctl debug`:

```
newline_lookahead () at src/scanner.c:2583
2583                ((Newline *) 0)->indent = 5;
(gdb) bt
#0  newline_lookahead () at src/scanner.c:2583
#1  0x00007ffff7a0740e in newline_start () at src/scanner.c:2604
#2  scan () at src/scanner.c:2646
#3  eval () at src/scanner.c:2684
#4  tree_sitter_haskell_external_scanner_scan (payload=<optimized out>, lexer=<optimized out>,
    valid_symbols=<optimized out>) at src/scanner.c:2724
#5  0x0000555555772488 in ts_parser.lex ()
```

For more control, start `gdb tree-sitter`, run `run test -f 'some test'`, and
set `break tree_sitter_haskell_external_scanner_scan`.

Disable optimizations with `tree-sitter test --debug-build`.

#### Tracing

The `test` and `parse` commands can emit detailed parse traces in two modes.

`tree-sitter test --debug` prints every lexer step and shift/reduce action to
stderr.

`tree-sitter test --debug-graph` writes an HTML graph of each step (requires
`graphviz`).

[tree-sitter]: https://github.com/tree-sitter/tree-sitter
[ref]: https://www.haskell.org/onlinereport/haskell2010/haskellch10.html
[ext]: https://downloads.haskell.org/~ghc/latest/docs/html/users_guide/exts/table.html
[cli]: https://github.com/tree-sitter/tree-sitter/tree/master/cli
[grammar-docs]: https://tree-sitter.github.io/tree-sitter/creating-parsers#writing-the-grammar
[ci]: https://img.shields.io/github/actions/workflow/status/tree-sitter/tree-sitter-haskell/ci.yml?logo=github&label=CI
[discord]: https://img.shields.io/discord/1063097320771698699?logo=discord&label=discord
[matrix]: https://img.shields.io/matrix/tree-sitter-chat%3Amatrix.org?logo=matrix&label=matrix
[npm]: https://img.shields.io/npm/v/tree-sitter-haskell?logo=npm
[crates]: https://img.shields.io/crates/v/tree-sitter-haskell?logo=rust
[pypi]: https://img.shields.io/pypi/v/tree-sitter-haskell?logo=pypi&logoColor=ffd242

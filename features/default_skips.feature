Feature: Default skip policy
  abcop reviews production code; declarative wiring, vendored trees and
  generated material are pruned unless explicitly targeted.

  Scenario: Framework route tables are not review surface
    Given a Rails application with "config/routes.rb" and "config/routes/api.rb"
    When a default walk or an "--mr" scope runs
    Then no diagnostics are reported for route tables
    And other files under the same tree are analysed normally

  Scenario: Explicit targeting opts back in
    Given the file "config/routes.rb"
    When abcop is invoked as "abcop config/routes.rb"
    Then exactly that file is analysed and its findings are reported

  Scenario: Similar boilerplate stays out of scoped runs
    Given an engine with "engines/billing/config/routes.rb"
    When an MR scope includes that path among changed files
    Then it is dropped from analysis like any other route table

  Scenario: Scoped runs keep code rules but drop route tables
    Given a branch whose changed files include tests and "config/routes.rb"
    When an "--mr" scope resolves its file list
    Then UsedOnce and NeverUsed still run on the test files
    And AbcSize reports changed methods over the method ceiling
    And ModuleAbcSize re-scores from changed methods only
    And route tables are excluded entirely
    And the goal is compact reviews that stay within the MR's task scope

  Scenario: Fixture sample trees stay out of scoped runs
    Given a branch whose changed files include "tests/fixtures/cops/x.rb"
    When an "--mr" or uncommitted scope resolves its file list
    Then that fixture path is dropped from analysis
    And real test files outside fixtures/ are still analysed
    And naming "tests/fixtures" on the CLI (without a scope flag) opts back in

  Scenario: Scoped ModuleAbcSize uses changed methods, AbcSize stays per-method
    Given a production module whose full ABC exceeds the module ceiling
    And the branch changed only a few lines inside one medium method
    When an "--mr" scope runs
    Then no ModuleAbcSize finding is reported for that file
    But AbcSize still reports if that changed method exceeds --max-abc
    And when the diff intersects methods whose ABC sum exceeds --max-module-abc
    Then the ModuleAbcSize warning is reported with that changed-method total

  Scenario: Default invocation prefers uncommitted work
    Given uncommitted edits against HEAD and commits on the branch
    When abcop runs with no scope flags
    Then only the uncommitted edits are analysed
    And a note announces the narrowed scope

  Scenario: Default invocation covers branch work on a clean tree
    Given commits on the branch and a clean working tree
    When abcop runs with no scope flags
    Then the branch's committed changes are analysed

  Feature: Ruby shorthand hash arguments count as reads
    Ruby 3 allows `foo(user:, strict: false)` where a value-less key reads
    the identically named local variable.

    Scenario: NeverUsed respects shorthand references
      Given a local assigned once and referenced only as "user:" in a call
      Then abcop reports no NeverUsed offense for that local

    Scenario: UsedOnce recognizes shorthand as the single read
      Given a pure literal bound once and read exactly once via "x:"
      Then abcop reports the UsedOnce inline candidate as usual

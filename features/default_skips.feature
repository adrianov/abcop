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
    And AbcSize and ModuleAbcSize follow --size-gate (default both: silent under 100 changed lines)
    And route tables are excluded entirely
    And the goal is compact reviews that stay within the MR's task scope

  Scenario: Size gate suppresses AbcSize and ModuleAbcSize on small scoped diffs
    Given a production module whose method and module ABC exceed the ceilings
    And the branch changed only 4 lines of it
    When an "--mr" scope runs with the default "--size-gate both"
    Then no AbcSize or ModuleAbcSize finding is reported for that file
    But when the diff touches at least 100 lines of it
    Then both size findings are reported

  Scenario: Size gate can target specs only or be disabled
    Given the same oversized production module with a 4-line diff
    When "--size-gate specs" is set
    Then production size findings still report
    And spec trees stay silent until a 100-line diff
    When "--size-gate none" is set
    Then every intersecting AbcSize and ModuleAbcSize finding reports

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

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
    Then AbcSize, UsedOnce and NeverUsed still run on the test files
    And only ModuleSize exempts test trees by default
    And route tables are excluded entirely
    And the goal is compact reviews that stay within the MR's task scope

  Scenario: ModuleSize gates scoped reviews only on refactor-scale diffs
    Given a 228-line production module where the branch changed 4 lines
    When an "--mr" scope runs
    Then no ModuleSize warning is reported for that module
    But when the diff touches at least 100 lines of it
    Then the ModuleSize warning is reported

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

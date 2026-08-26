Feature: Python language support
  abcop scores Python sources with the same four rules it applies to
  Ruby and Rust: Metrics/AbcSize per function, UsedOnce, NeverUsed and
  ModuleSize.

  Background:
    Given a Python file "sample.py"

  Scenario: Named functions are scored as units
    When abcop analyses the file
    Then every named def produces one AbcSize offense candidate
    And nested defs score independently without double-counting in the parent
    And lambda bodies roll into the enclosing unit

  Scenario: ABC counting mirrors the cross-language semantics
    When the file contains assignments, calls, operators, branches,
      loops, comprehension clauses, except clauses, match arms,
      comparisons and boolean operators
    Then assignments count toward A including loop targets
    And calls, attribute reads, subscripts, arithmetic/bitwise/unary
      operators and f-string interpolations count toward B
    And if/elif/while/for, comprehension for/if clauses, ternaries,
      each except clause, each match arm, comparisons and and/or count toward C

  Scenario: UsedOnce flags inlinable bindings
    Given a local bound once by a plain assignment or walrus with a pure RHS
    When the binding is written on a straight-line path and read exactly
      once afterwards
    Then abcop reports UsedOnce at the write position

  Scenario: UsedOnce rejects unsafe candidates
    Given a binding whose RHS calls, reads attributes or references locals
    When the binding is also reassigned, augmented, written under control
      flow, or read before its write
    Then abcop does not report it

  Scenario: NeverUsed reports dead writes
    Given a local that is assigned but never read
    Then abcop reports NeverUsed once at the first write
    And tuple-unpacking targets are reported individually

  Scenario: Protocol bindings are exempt from UsedOnce candidacy
    Given names bound by function parameters, for-loop targets,
      import statements, match captures, or underscore prefixes
    Then abcop never reports them as used-once candidates

  Scenario: Suppression comments work in Python sources
    Given a line carrying "# rubocop:disable Metrics/AbcSize" style directives
    Then AbcSize offenses on that line are suppressed like in Ruby

  Scenario: ModuleSize applies to Python files
    Given a production Python module of 200 lines or more
    Then abcop reports a ModuleSize warning

Feature: Go and PHP language support
  The same four rules run on Go (.go) and PHP (.php) sources.

  Scenario: Named functions and methods are units
    When abcop analyses Go or PHP sources
    Then named declarations produce AbcSize candidates
    And anonymous/arrow/closure bodies roll into the enclosing unit

  Scenario: Language operators follow the shared B/C split
    When expressions contain calls, arithmetic and condition logic
    Then calls and arithmetic/bitwise operators count toward B
    And branches, case/match arms, catches, comparisons and logic count toward C

  Scenario: Loop heads are protocol bindings
    Given variables bound by foreach/range heads
    Then they never qualify as UsedOnce candidates
    And unused loop keys are not reported as dead writes

  Scenario: Scoped runs treat Go and PHP like first-class languages
    Given changes in Go or PHP files within an MR scope
    Then all applicable rules run with suppression directives honored

Feature: Java language support
  The same four rules run on Java (.java) sources.

  Scenario: Methods and constructors are units
    When abcop analyses Java sources
    Then method and constructor declarations produce AbcSize candidates
    And lambdas roll into the enclosing unit

  Scenario: Switch labels and update expressions follow the shared spec
    When expressions contain invocations, increments and condition logic
    Then invocations and arithmetic count toward B
    And each switch label, catch clause and comparison counts toward C
    And i++ / --i rewrite a variable exactly like other assignments

  Scenario: Members and qualified names are never variable reads
    Given field accesses, method names and package-qualified types
    Then only genuine local identifiers are tracked for the variable rules

Feature: C# language support
  The same four rules run on C# (.cs) sources.

  Scenario: Methods and constructors are units
    When abcop analyses C# sources
    Then method and constructor declarations produce AbcSize candidates
    And lambdas roll into the enclosing unit

  Scenario: Assignments to undeclared names are field writes
    Given a bare identifier assigned that no visible local introduced
    Then the assignment contributes operand reads only
    And no false NeverUsed is reported for the class field

  Scenario: Protocol and member rules match the other backends
    Then foreach heads and catch declarations never become candidates
    And member name slots are not variable reads

Feature: JavaScript and TypeScript variable rules
  UsedOnce and NeverUsed run on the JS/TS family on top of AbcSize.

  Scenario: Member slots are not variables
    Given property accesses like "it.length"
    When the collector walks declarations and expressions
    Then only genuine identifiers register as reads or writes

  Scenario: Loop heads are protocol
    Given for-in/for-of control variables
    Then they never become UsedOnce candidates or dead-write reports

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

Feature: C, C++ and Objective-C language support
  abcop scores the plain-C-family grammars with the same four rules it
  applies to every other backed language: Metrics/AbcSize per function,
  UsedOnce, NeverUsed and ModuleAbcSize. All three share tree-sitter's C core
  shapes -- reads are `identifier`, declarations carry `init_declarator`
  with an optional initializer, assignments use the JS-shaped
  left/operator/right fields -- so one collector plus three spec tables
  serves them.

  Background:
    Given a C-family source file

  Scenario: Named functions and methods are scored as units
    When abcop analyses the file
    Then every function definition produces one AbcSize offense candidate
    And Objective-C method definitions produce their own units

  Scenario: ABC counting mirrors the cross-language semantics
    When the file contains assignments, calls, arithmetic operators,
      branches, loops, switch arms, comparisons and boolean operators
    Then assignments count toward A
    And calls and message sends count toward B
    And control-flow and comparison operators count toward C

  Scenario: UsedOnce flags inlinable locals
    Given a local defined by an init declarator with a pure literal or
      operator-composition right-hand side
    When the local is written on a straight-line path and read exactly once
    Then abcop reports UsedOnce at the write position

  Scenario: UsedOnce rejects unsafe candidates
    Given a local whose right-hand side is impure (a call, another local,
      a member read, a lambda, a brace initializer)
    Or the local is reassigned, augmented or incremented before its single
      read
    Then abcop reports no UsedOnce offense for it

  Scenario: NeverUsed reports dead writes including bare definitions
    Given a local declared but never read (`int orphan;` counts)
    Then abcop reports NeverUsed once at the declaration

  Scenario: Member slots are not variable reads
    Given struct fields, Objective-C properties and messages
    Then reads via `p.px`, `self.width` or `[W new]` never register as
      reads of same-named locals
    And writing `p.px = g` also reads `p` itself -- object evaluations are
      real reads, so p stays free of false NeverUsed/UsedOnce

  Scenario: Globals stay out of single-file analysis
    Given file-scope statics, globals and enum constants
    Then reads of them resolve to nothing and produce no findings

  Scenario: Loop heads are protocol
    Given a `for (int i = 0; ...; i++)` head or a C++ range-for binding
    Then the loop variable is never tracked
    And no inlining suggestion fires even when it is written once and read
      once inside the loop

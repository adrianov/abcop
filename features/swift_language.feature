Feature: Swift language support
  abcop scores Swift sources with the same four rules it applies to
  Ruby, Rust and the JavaScript/TypeScript family: Metrics/AbcSize per
  function, UsedOnce, NeverUsed and ModuleSize. Swift reuses the shared
  scope-model engine (the JS/TS backend's collector) with a Swift-specific
  spec: bindings are `property_declaration`/`value_binding_pattern`, reads
  are `simple_identifier`, assignments are `assignment` (plain `=` and
  compound `+=`-style).

  Background:
    Given a Swift file "sample.swift"

  Scenario: Named functions and initializers are scored as units
    When abcop analyses the file
    Then every `func` and `init` declaration produces one AbcSize offense candidate
    And nested functions score independently without double-counting in the parent
    And closure expressions roll into the enclosing unit

  Scenario: ABC counting mirrors the cross-language semantics
    When the file contains assignments, calls, infix operators, branches,
      loops, switch arms, comparisons and boolean operators
    Then assignments count toward A
    And calls and message sends count toward B
    And control-flow and comparison operators count toward C

  Scenario: UsedOnce flags inlinable bindings
    Given a `let` binding with a pure literal right-hand side
    When the binding is written on a straight-line path and read exactly
      once afterwards
    Then abcop reports UsedOnce at the write position

  Scenario: UsedOnce rejects unsafe candidates
    Given a binding whose right-hand side calls, reads attributes, references
      other locals, or crosses an assignment to the same name
    When the binding is reassigned, augmented, or read before its write
    Then abcop reports no UsedOnce offense for it

  Scenario: NeverUsed reports dead writes
    Given a local that is assigned but never read
    Then abcop reports NeverUsed once at the first write

  Scenario: Member field reads are not variable reads
    Given a `let x = 1` followed by `return self.x`
    Then abcop reports NeverUsed for the local `x` (never read)
    And abcop reports no UsedOnce offense (the `self.x` member read is not
      a read of the same-named local)

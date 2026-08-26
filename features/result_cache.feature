Feature: Single-database result cache
  Repeated scans of unchanged files are served from one embedded
  key-value database instead of many per-file JSON entries.

  Background:
    Given the cache directory "$XDG_CACHE_HOME/abcop" (or $ABCOP_CACHE_DIR)

  Scenario: Unchanged files are served warm
    When abcop scans a tree twice with unchanged contents and settings
    Then the second run reads every result from the cache database
    And both runs report identical diagnostics apart from timing

  Scenario: Cache keys prevent stale results
    Given entries keyed by a hash of file contents, tool version,
      rule revision, threshold, selected checks and path
    When any component changes
    Then the old entry can no longer be served

  Scenario: The store is one database file
    When abcop stores results
    Then a single "cache.redb" file appears in the cache directory
    And no per-entry JSON files are created

  Scenario: Legacy JSON entries are removed on open
    Given the cache directory contains "*.json" files from an older version
    When abcop opens the cache successfully
    Then those legacy files are deleted best-effort
    And non-cache files in the directory are left untouched

  Scenario: Corrupt entries fail safe
    Given an undecodable entry in the database
    When abcop looks up its key
    Then the lookup is a miss and analysis proceeds normally

  Scenario: Pruning keeps the newest entries
    Given more than 20000 entries in the cache
    When abcop prunes
    Then only the 20000 most recently stored entries remain

  Scenario: Disabling the cache
    When abcop runs with --no-cache
    Then nothing is read from or written to the cache database

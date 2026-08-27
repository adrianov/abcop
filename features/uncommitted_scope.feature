Feature: Uncommitted-only scope
  `--uncommitted` narrows the scan to exactly the working tree: unstaged
  edits, staged files and untracked files against HEAD. The branch's
  committed work is out of scope, and outside a repository the run fails
  loudly instead of silently widening.

  Scenario: Working-tree work is selected in all three states
    Given a repository with committed branch work
    And one file edited but unstaged, one file staged, one file untracked
    When abcop runs with "--uncommitted"
    Then all three uncommitted files are analysed
    And no committed file that the working tree does not touch is analysed

  Scenario: Committed branch work stays out
    Given a branch with commits past its fork point
    And uncommitted edits against HEAD
    When abcop runs with "--uncommitted"
    Then only the uncommitted edits are analysed
    And the branch's committed changes are not added to the scope

  Scenario: No repository fails loudly
    Given a directory that is not a git repository
    When abcop runs with "--uncommitted"
    Then the run exits with the scope-error code
    And the full tree is not silently scanned instead

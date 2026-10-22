Feature: Verified sites with one shared policy
  Every unit checks the bytes at the game addresses it uses. A call site
  accepts another component's entry detour (hooks chain through it); a hook
  site or an anchor accepts only the retail bytes and never stomps a foreign
  jump. Anything else is another executable: fail closed.

  Background:
    Given the retail image with a call head, a hook head, an anchor and a slice anchor

  Scenario: The retail image passes
    Then verification passes with no findings

  Scenario: A foreign rel32 detour is accepted at the call site and refused at the hook site
    When another component writes a "jmp rel32" detour at the head to executable memory outside the image
    And the anchor byte changes
    Then 2 sites fail and 2 carry a foreign detour
    And the findings are "call head: foreign", "hook head: occupied" and "anchor: bytes"

  Scenario Outline: Detours that are not recognised are mismatches
    Then classifying a head carrying <detour> gives "<verdict>"

    Examples:
      | detour                                      | verdict  |
      | the retail bytes                            | retail   |
      | a rel32 jump to executable memory           | foreign  |
      | an indirect jump through a readable slot    | foreign  |
      | an indirect jump through an unreadable slot | mismatch |
      | a rel32 jump into the image                 | mismatch |
      | a rel32 jump to non-executable memory       | mismatch |
      | a rel32 jump to address zero                | mismatch |
      | a rel32 jump with a changed byte after it   | mismatch |
      | a shorter byte run                          | mismatch |
      | other bytes                                 | mismatch |
      | a rel32 jump over a five-byte head          | mismatch |
      | a rel32 jump with no foreign code mapped    | mismatch |

  Scenario: Only call sites admit a foreign detour
    Then call sites admit retail and foreign bytes, hook sites and anchors only retail

  Scenario: Sites outside a short or unreadable image fail
    Then an image of 0x150 bytes fails all 4 sites, some outside the image
    And an unreadable image fails every site
    And a site longer than 512 bytes is outside the comparable range
    And empty or oversized sites fail the table check

  Scenario: Problems and bytes read clearly in log lines
    Then the problems read "outside the loaded image", "bytes differ from the verified build" and "already hooked by another component (jmp [abs32] -> 0x00001234); not overwritten"
    And bytes display as "0f b6 45"

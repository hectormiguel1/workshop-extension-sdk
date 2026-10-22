Feature: Save lifecycle events
  The host tells a unit when the game saves, loads or starts a new game.
  Handlers are advisory, run on the game's thread and never unwind into it.

  Scenario: Event kinds and masks follow the header
    Then every event kind has its ABI value and mask
    And a slot or profile is reported only when the host flags it valid

  Scenario: Subscribed handlers receive their events
    Given a fake FINAL FANTASY XIII host
    When the unit subscribes to saved and new-game events and a panicking loaded handler
    Then the adapter reports all events over 100 slots
    And a saved event for slot 2 and a new game reach the handler, a save begin does not
    And the panicking handler answers -1000 instead of unwinding
    And null and short events are refused

  Scenario: Registration arguments and host failures
    Given a fake FINAL FANTASY XIII host
    Then an empty mask, an unknown event bit and an empty ID are refused
    And a conflicting adapter hook is reported
    And a table without entries is unavailable

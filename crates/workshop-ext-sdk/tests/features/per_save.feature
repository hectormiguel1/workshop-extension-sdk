Feature: Per-save settings
  A unit's settings follow the player's save slot: host storage keyed by
  the save events' profile and slot, never the game's saves. Everything is
  off at process start, menu edits stay in memory until a save completes,
  and a slot without a record loads as defaults with Enabled off.

  Background:
    Given a fake FINAL FANTASY XIII host
    And a per-save store over 8 slots

  Scenario: Process start is off; menu edits stay in memory
    Then the per-save store opened fresh
    And the per-save settings are Enabled off, Level 3
    When the menu sets Enabled on and Level 7
    Then the per-save region is unwritten

  Scenario: A save keeps what SAVE_BEGIN captured; loads restore each slot
    When the menu sets Enabled on and Level 7
    And save operation 10 begins for slot 2 of profile 1
    And the menu sets Enabled on and Level 9
    And save operation 10 completes for slot 2 of profile 1
    Then the per-save event answered ok
    And slot 2 of profile 1 is stored on
    And the per-save store is ready
    And the per-save log mentions "slot 2 saved"
    When slot 5 of profile 1 is loaded
    Then the per-save settings are Enabled off, Level 3
    And the per-save log mentions "slot 5 has no record: defaults, off"
    When slot 2 of profile 1 is loaded
    Then the per-save settings are Enabled on, Level 7
    And the per-save log mentions "slot 2 restored (on)"

  Scenario: The next session reads the stored region
    When the menu sets Enabled on and Level 8
    And slot 0 of profile 1 is saved as operation 1
    And the next session opens the stored region over 8 slots
    Then the per-save store opened stored
    And the per-save settings are Enabled off, Level 3
    When slot 0 of profile 1 is loaded
    Then the per-save settings are Enabled on, Level 8

  Scenario: A failed save is discarded; unknown operations and kinds are ignored
    When the menu sets Enabled on and Level 4
    And save operation 20 begins for slot 1 of profile 1
    And save operation 20 fails for slot 1 of profile 1
    Then the per-save event answered ok
    And the per-save log mentions "slot 1 save failed; capture discarded"
    And the per-save region is unwritten
    When save operation 99 completes for slot 1 of profile 1
    Then the per-save event answered not-found
    And the per-save log mentions "save operation 99 unknown; ignored"
    When save operation 21 begins for slot 1 of profile 1
    And save operation 21 begins for slot 1 of profile 1
    Then the per-save event answered invalid
    When save operation 21 completes for slot 3 of profile 1
    Then the per-save event answered invalid
    And the per-save log mentions "changed slot; discarded"
    When an event of unknown kind 77 arrives
    Then the per-save event answered invalid
    And the per-save log mentions "unknown save event kind 77"

  Scenario: Captures are bounded and need a slot, a profile and an operation
    When 64 saves begin for slot 1 of profile 1
    And save operation 5000 begins for slot 1 of profile 1
    Then the per-save event answered limit
    And 64 captures are pending
    When a save begins without a profile
    Then the per-save event answered invalid
    When a save begins without an operation id
    Then the per-save event answered invalid
    And the per-save log mentions "save without an operation id"

  Scenario: A load without a valid identity is off and never slot 0
    When the menu sets Enabled on and Level 9
    And slot 0 of profile 1 is saved as operation 1
    And a load arrives without a slot
    Then the per-save event answered invalid
    And the per-save settings are Enabled off, Level 3
    And the per-save log mentions "load without a valid slot/profile: defaults, off"
    When the menu sets Enabled on and Level 9
    And a load arrives without a profile for slot 0
    Then the per-save settings are Enabled off, Level 3

  Scenario: A new game keeps the title-screen choices; quitting to title changes nothing
    When slot 3 of profile 1 is loaded
    And the menu sets Enabled on and Level 5
    Then the per-save settings are Enabled on, Level 5
    When a new game starts
    Then the per-save event answered ok
    And the per-save settings are Enabled on, Level 5
    And the per-save log mentions "new game: settings kept from the title screen"
    And the per-save region is unwritten

  Scenario: Profiles are kept apart up to the directory capacity
    When the menu sets Enabled on and Level 2
    And slot 1 of profile 1 is saved as operation 1
    And the menu sets Enabled off and Level 6
    And slot 1 of profile 2 is saved as operation 2
    And slot 1 of profile 1 is loaded
    Then the per-save settings are Enabled on, Level 2
    When slot 1 of profile 2 is loaded
    Then the per-save settings are Enabled off, Level 6
    When 14 more save profiles each save slot 0
    And slot 0 of profile 99 is saved as operation 500
    Then the per-save log mentions "save profile directory full; slot 0 not persisted"
    And the per-save store is ready

  Scenario: A revision conflict retries once; a changed header stops persisting
    When slot 0 of profile 1 is saved as operation 1
    And another writer bumps the region revision
    And the menu sets Enabled on and Level 4
    And slot 1 of profile 1 is saved as operation 2
    Then slot 1 of profile 1 is stored on
    When another writer changes the region header
    And slot 2 of profile 1 is saved as operation 3
    Then the per-save store is failed
    And the per-save log mentions "slot 2 write failed"
    When slot 3 of profile 1 is saved as operation 4
    Then the per-save log mentions "slot 3 not persisted (store is Failed)"
    When slot 1 of profile 1 is loaded
    Then the per-save settings are Enabled off, Level 3

  Scenario: Host write and flush failures
    When the host flush fails with -7
    And slot 0 of profile 1 is saved as operation 1
    Then the per-save log mentions "flush failed (-7)"
    And slot 0 of profile 1 is stored off
    When the host refuses storage writes with -7
    And slot 1 of profile 1 is saved as operation 2
    Then the per-save store is failed
    And the per-save log mentions "write failed (-7)"

  Scenario Outline: A region another layout wrote is never read or written
    When the menu sets Enabled on and Level 8
    And slot 0 of profile 1 is saved as operation 1
    And the next session finds the region damaged: <damage>
    Then the per-save store opened foreign
    And the per-save log mentions "<why>"
    When slot 0 of profile 1 is saved as operation 2
    And slot 0 of profile 1 is loaded
    Then the per-save settings are Enabled off, Level 3
    And the per-save region bytes are unchanged
    And the import is refused

    Examples:
      | damage    | why                            |
      | schema    | region schema unknown          |
      | short     | region shorter than its header |
      | magic     | not a per-save region          |
      | layout    | settings layout changed        |
      | slots     | slot count changed             |
      | truncated | region truncated               |

  Scenario: Without host storage settings last for the session
    Given a per-save store without host storage
    Then the per-save store opened without-storage
    And the per-save store is memory
    When the menu sets Enabled on and Level 8
    And slot 0 of profile 1 is saved as operation 1
    Then the per-save log mentions "slot 0 not persisted (store is Memory)"
    And a per-save store over 0 slots is refused
    And a host that cannot describe the region fails the store

  Scenario: Damaged records load as defaults, off
    When the menu sets Enabled on and Level 8
    And slot 2 of profile 1 is saved as operation 1
    And the stored record of slot 2 is damaged
    And slot 2 of profile 1 is loaded
    Then the per-save settings are Enabled off, Level 3
    And the per-save log mentions "slot 2 record rejected (Checksum)"
    When the stored records are cut off
    Then slot 2 of profile 1 is stored absent
    And the per-save log mentions "slot 2 unreadable"

  Scenario: A legacy file is imported once, into a region never written
    When legacy records are imported: 2 profiles with 3 slots each, Enabled off
    Then 6 records were imported
    And the per-save store is ready
    And the per-save log mentions "imported 6 record(s) over 2 save profile(s)"
    And the per-save region is marked imported
    And the import is refused
    When slot 1 of profile 2 is loaded
    Then the per-save settings are Enabled off, Level 6

  Scenario: A large import goes out in bounded batches; a failed batch stops persisting
    Then a wide import over 200 slots of 3 profiles writes several 64 KiB batches
    When the host refuses storage writes with -7
    And legacy records are imported: 1 profiles with 1 slots each, Enabled on
    Then the per-save store is failed
    And the per-save log mentions "import failed (-7)"

  Scenario: Deferred saves wait for the writer; a load finishes them first
    When the store defers writes to its writer
    And the menu sets Enabled on and Level 3
    And slot 1 of profile 1 is saved as operation 1
    Then 1 writes are queued
    And the per-save region is unwritten
    When the writer writes the queue
    Then 0 writes are queued
    When the menu sets Enabled on and Level 6
    And slot 2 of profile 1 is saved as operation 2
    And slot 2 of profile 1 is loaded
    Then 0 writes are queued
    And the per-save settings are Enabled on, Level 6

  Scenario: Records and headers carry their identity and reject damage
    Then a record of slot 4 decodes for slot 4 only and at its own version
    And a never-saved record reads as no record and a short one is refused
    And a header round-trips and rejects a bad magic, checksum, version and directory
    And layout hashes follow keys and ranges, not defaults
    And records larger than the contract are refused

  Scenario: The params adapter packs settings and restores them through their setters
    Then the params adapter packs Enabled on, Level 8, Gil 999 into 2 bytes and back
    And a payload of another layout restores defaults, off

  Scenario: Params clamp, report changes and bind to menu controls
    Then a param over 10..2000 clamps 5000 to 2000 and reports the change once
    And menu values round half up, negatives read 0 and NaN is ignored
    And param controls write their params through the menu callbacks

  Scenario: Import edge cases
    Then an import skips slots past the count, other payload sizes and profiles past capacity
    And an import of nothing writes nothing
    And a flush failure after an import is logged
    And stores and adapters also build at run time

  Scenario: The writer thread sleeps until a save is queued
    Then a writer thread handles a deferred save of a store without storage

Feature: Host storage regions
  Mod state lives in the host's mod-state.dat under a stable region ID,
  never in the game's saves. A new region reads as absent until written;
  writes are atomic batches checked against the revision.

  Scenario: A region round-trips through whole and ranged writes
    Given a fake FINAL FANTASY XIII host
    When the unit registers the region "example.state" in init
    Then the region reads as never written
    When the unit writes "hello" with schema 2
    Then the region holds "hello" with schema 2 at revision 1
    And bytes 3..5 read "lo" and bytes 4..6 are refused
    And a write against revision 0 is refused as a conflict
    When the unit writes "J" at 0 and "!" at 5 in a 6-byte batch
    Then the region holds "Jello!" with schema 3 at revision 2
    When the unit empties the region
    Then the region holds "" with schema 3 at revision 3
    And more than 256 ranges are refused as over the limit
    And schema 0 is refused
    And a flush reaches the host

  Scenario Outline: Region IDs are validated
    Then the region ID "<id>" is <validity>

    Examples:
      | id                         | validity |
      | example.ff13.zone_warp | valid    |
      | a-b_c.9                    | valid    |
      | no spaces                  | invalid  |
      |                            | invalid  |

  Scenario: Registration conflicts and host failures are reported
    Given a fake FINAL FANTASY XIII host
    When the unit registers the region "example.state" in init
    Then registering "example.state" again is refused as a conflict
    And a host in recovery refuses registration and an IO error fails the write
    And a short info answer is a decode failure
    And a handle the host never issued is refused
    And a table without entries refuses every call but flush
    And a zero handle, a failing flush and an oversized region are reported

  Scenario: Storage without the service lasts for the session
    Given a fake FINAL FANTASY XIII host
    When the context's size is prefix
    Then init sees no storage, settings or save events service

  Scenario: Raw tables can be registered outside the init scope helpers
    Given a fake FINAL FANTASY XIII host
    Then raw storage, settings and save event tables register through copies

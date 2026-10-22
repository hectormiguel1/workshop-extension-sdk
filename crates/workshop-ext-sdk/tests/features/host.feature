Feature: Host context validation and services
  A unit sees the host only through a validated context: another ABI
  version or a context smaller than the version 1 prefix is refused before
  any other field is read, and the optional service tables exist only when
  the advertised size covers them.

  Scenario Outline: Unusable contexts are refused with the unit's log text
    Given a fake FINAL FANTASY XIII host
    When the context's <field> is <value>
    Then the context is refused with "<reason>"

    Examples:
      | field   | value | reason                               |
      | pointer | null  | host context is null                 |
      | version | 2     | host ABI version 2 (need 1)          |
      | size    | 8     | host context size 8 is too small     |

  Scenario Outline: Requirements are checked in order
    Given a fake FINAL FANTASY XIII host
    When the context's <field> is <value>
    Then requiring FINAL FANTASY XIII with log and VFS and an image fails with "<reason>"

    Examples:
      | field     | value | reason                            |
      | game      | 2     | game 2 is not FINAL FANTASY XIII  |
      | vfs_read  | null  | host service vfs_read is missing  |
      | log       | null  | host service log is missing       |
      | game_base | 0     | host context has no game image    |

  Scenario: A complete context meets every requirement
    Given a fake FINAL FANTASY XIII host
    Then every requirement is met
    And every service names its context field

  Scenario: Merged-container files are read within a limit
    Given a fake FINAL FANTASY XIII host
    And the merged containers hold "vfx/x.xfv" with "package"
    Then reading "vfx/x.xfv" with a limit of 64 bytes gives "package"
    And reading "vfx/x.xfv" with a limit of 3 bytes is refused as too large (7 bytes)
    And reading "missing.bin" reports host status -2
    And a host whose read fails reports host status -3
    And a host without VFS services reports the read unavailable

  Scenario: Log lines reach the host at their levels
    Given a fake FINAL FANTASY XIII host
    When the unit logs at every level
    Then the host received the lines "plain", "e 1", "w 2" and "i 3"
    And a context without a log drops lines silently

  Scenario: Service tables are guarded by size and version
    Given a fake FINAL FANTASY XIII host
    Then storage, settings and save events are all advertised
    When the context's size is prefix
    Then storage, settings and save events are all absent

  Scenario Outline: Malformed service tables are refused
    Given a fake FINAL FANTASY XIII host
    When the host advertises a <table> table with <defect>
    Then the service is refused with "<reason>"

    Examples:
      | table       | defect                | reason                            |
      | storage     | version 2             | storage service version/size      |
      | storage     | size 8                | storage service version/size      |
      | storage     | no read_at            | storage service table incomplete  |
      | settings    | no register_provider  | settings service version/size     |
      | settings    | version 0             | settings service version/size     |
      | settings    | size 4                | settings service version/size     |
      | save events | no register_callback  | save events service version/size  |
      | save events | version 7             | save events service version/size  |
      | save events | size 4                | save events service version/size  |

  Scenario: A stored host is found again after init
    Given a fake FINAL FANTASY XIII host
    Then a host slot remembers the context until cleared

  Scenario: The context layout is the C header's
    Given a fake FINAL FANTASY XIII host
    Then the x86 context offsets match the header's static assertions
    And a prefix-sized context reads its service tables as absent

  Scenario: Log lines are built on the stack without allocating
    Then a default line keeps 192 bytes of whole characters
    And a 3-byte line keeps "abc" of "abcd" and "e"

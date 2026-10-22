Feature: The unit boundary
  A unit exports exactly init and shutdown. init validates the context
  before the unit runs, maps the unit's result to the host's codes and
  catches panics at the FFI boundary.

  Scenario: The exports validate, run and shut down
    Given a fake FINAL FANTASY XIII host
    Then init returns 0 and the unit logged "started on 1"
    And init of a null context returns 1
    And init of a version 2 context returns 1 without running the unit
    And init of an 8-byte context returns 1 without running the unit
    And shutdown runs the unit's shutdown, with or without one, and contains panics

  Scenario Outline: Results and panics become host codes
    Given a fake FINAL FANTASY XIII host
    Then an init that <outcome> returns <code>

    Examples:
      | outcome                    | code |
      | refuses                    | 2    |
      | fails a hook               | 3    |
      | returns code 77            | 77   |
      | returns code 0 as an error | 2    |
      | panics with a message      | 4    |
      | panics with a number       | 4    |
      | panics with a static text  | 4    |

  Scenario: Init errors convert and display
    Then init errors read "init returned 2" and convert from hook and service failures

  Scenario: The init scope hands out init-only services
    Given a fake FINAL FANTASY XIII host
    Then an init using storage, settings, save events and hooks succeeds
    And a second registration of its region is a conflict
    And a malformed storage table refuses init with code 2
    And without MinHook services the install fails at create and init returns 3
    And a prefix-only context offers no optional service

  Scenario: Built units export exactly init and shutdown
    Then a PE32 image exporting init and shutdown is a unit
    And images exporting more, nothing, another machine or a broken table are not

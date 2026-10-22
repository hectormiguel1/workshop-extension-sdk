Feature: Settings providers
  A unit's Mod Settings page is v1 descriptor metadata: stable IDs, kinds,
  bounded strings and extension chains, built once for the process. Every
  callback is guarded; a malformed descriptor fails loudly in the unit's
  own tests.

  Scenario: A provider has the v1 wire layout
    Given the example provider with every control kind
    Then it has 7 controls of kinds status, section, checkbox, integer, float, combo, button
    And its metadata reads "example.menu", "Example", "Tests", "1.0.0", "Help." and "https://example.invalid"
    And the status row reports "ready " within 8 bytes, never splitting a character
    And the section is experimental and collapsed, the checkbox visible and the integer prefixed "Lv "
    And the combo offers "A", "B" and "C"

  Scenario: Menu writes are converted, rounded and clamped
    Given the example provider with every control kind
    Then the checkbox turns on at 0.6 and off at 0.4, ignoring NaN
    And the integer rounds 7.6 to 8, clamps 99 to 10 and ignores infinity
    And the float clamps 5 to 2 and ignores NaN
    And the combo rounds 1.4 to 1 and clamps -3 to 0
    And pressing the button runs it once

  Scenario: Panicking callbacks never reach the game
    Given a provider whose callbacks all panic
    Then reads give 0, writes and presses are ignored and the status text is empty
    And a panicking state callback reads as visible

  Scenario Outline: Malformed descriptors fail loudly
    Then building <mistake> panics with "<message>"

    Examples:
      | mistake                  | message                                     |
      | a duplicate id           | settings: duplicate control id              |
      | an empty id              | settings: invalid string (too long or NUL)  |
      | a NUL in an id           | settings: invalid string (too long or NUL)  |
      | a 193-byte label         | settings: invalid string (too long or NUL)  |
      | an extension first       | settings: add a control first               |
      | two presentations        | settings: duplicate extension               |
      | a combo without options  | settings: invalid range or options          |
      | an integer step of 0     | settings: invalid range or options          |
      | an inverted float range  | settings: invalid range or options          |
      | 7 display decimals       | settings: invalid range or options          |
      | 257 controls             | settings: more than 256 controls            |

  Scenario: The declared provider answers matching queries and registers
    Then the declared provider answers XIII queries only, once built
    And a provider for any game answers every game
    And a provider whose build panics answers null
    And query contexts of another version or size are refused
    When a fake FINAL FANTASY XIII host registers the declared provider
    Then the host's menu sees 1 control
    And registering it again is refused as a conflict
    And an unavailable registry is reported

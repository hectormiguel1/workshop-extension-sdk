Feature: A unit's own menu texts
  The XIII menu adapter looks every provider string up in the game's ZTR
  resources by a hash key; a unit ships its strings in all eleven locales
  and checks its catalog and its built container against what is looked up.

  Scenario: Keys, placeholders and the strings a menu looks up
    Then the key of "Gil" is "$ms8b1998869903"
    And placeholders are the sorted single digits in braces
    And a provider's looked-up strings are its short name, labels, help, options and affixes without status rows

  Scenario: Catalog and container checks
    Then a complete catalog has no problems, a missing, empty or drifted entry is reported
    And a container dump missing a locale fails the check
    And the catalog is exported as escaped JSON lines

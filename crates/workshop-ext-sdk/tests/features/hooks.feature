Feature: All-or-nothing hook installs
  Hooks are created only in init, then enabled together. The first failure
  disables and removes every hook already created and clears its
  trampoline, so a unit is fully hooked or not at all.

  Scenario: A hook set installs enabled with trampolines
    Given a fake FINAL FANTASY XIII host
    When the unit installs 3 hooks
    Then 3 hooks are enabled and each detour holds its trampoline
    And disabling, enabling and removing a hook reach the host

  Scenario: A failed creation rolls back what was created
    Given a fake FINAL FANTASY XIII host
    And the host fails the 3rd hook creation
    When the unit installs 3 hooks
    Then the install fails with "hook create failed at 0x403000 (status -6); removed 2 hook(s)"
    And no hook remains and every trampoline is cleared

  Scenario: A failed enable rolls back every hook
    Given a fake FINAL FANTASY XIII host
    And the host fails enabling the 2nd hook
    When the unit installs 3 hooks
    Then the install fails with "hook enable failed at 0x402000 (status -6); removed 3 hook(s)"
    And no hook remains and every trampoline is cleared

  Scenario: A null trampoline is a failed creation
    Given a fake FINAL FANTASY XIII host
    And the host hands back null trampolines
    When the unit installs 3 hooks
    Then the install fails with "hook create failed at 0x401000 (status -6); removed 0 hook(s)"

  Scenario: Hook services must all be present
    Given a fake FINAL FANTASY XIII host
    When the context's hook_create is null
    Then hooks can be controlled but not created

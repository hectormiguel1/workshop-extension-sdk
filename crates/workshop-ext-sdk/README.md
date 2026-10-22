# workshop-ext-sdk

The SDK for **Crux extension units**: native DLL mods for the FINAL
FANTASY XIII trilogy (32-bit Windows; runs under Wine/Proton too). The
Crux host (`Crux.dll`) loads a unit that a mod's CrystalWorkshop Shard (`.cws`)
ships, from the Crux image PulseDepot merges (`active.crximg`), calls its `init` with a
service context (log, merged-container files, MinHook, persistent storage,
the shared Mod Settings menu, save events) and keeps it resident.

Licensed under either of MIT or Apache-2.0, at your option
(`LICENSE-MIT`, `LICENSE-APACHE`). No runtime dependency besides its own
derive crate.

## What a unit looks like

```rust
use workshop_ext_sdk::prelude::*;
use workshop_ext_sdk::sites;

const NEEDS: Requirements = Requirements {
    game: Some(abi::GAME_XIII),
    services: &[Service::Log],
    image: true,
};

workshop_ext_sdk::sites! {
    pub const SITES = {
        "BattleChara_GetHp" @ 0x0006_e2d0 = b"\x55\x8b\xec\x51\x89\x4d\xfc\xb9", call;
    }
}

fn start(init: &Init) -> InitResult {
    let host = init.host();
    host.require(&NEEDS).map_err(|_| InitError::UNUSABLE_CONTEXT)?;
    // ... verify SITES, register storage/settings, install hooks ...
    host.info(format_args!("my unit: ready"));
    Ok(())
}

workshop_ext_sdk::ff13_unit! { init: start }
```

`hello-unit` (in the repository's examples) is a complete unit with a verified call site, a
persisted `#[derive(PackedRecord)]` record and a settings page, plus tests
over the in-memory host.

## Building

```sh
cargo build -p hello-unit --target i686-pc-windows-gnu --profile host-dll
cargo run -p workshop-ext-sdk --example export_check -- \
    target/i686-pc-windows-gnu/host-dll/hello_unit.dll
```

* Target `i686-pc-windows-gnu`, `crate-type = ["cdylib"]` (add `"rlib"` to
  test the logic on your build machine).
* Keep `panic = "unwind"` in the shipping profile: `init`, `shutdown` and
  every SDK callback trampoline catch panics at the FFI boundary. With
  `panic = "abort"` a panic terminates the game.
* A unit exports exactly `init` and `shutdown`. No `DllMain` logic, no
  proxy exports; `export_check` verifies a built DLL.

## The pieces

| Module | What it gives a unit |
| --- | --- |
| `abi` | `#[repr(C)]` mirrors of `ff13_host.h`, `ff13_settings.h`, `ff13_save_events.h`; every struct and constant is compared against the headers by the crate's tests. |
| `host` | `Host`: a validated context copy (`Requirements`, `Unusable`), logging, merged-container reads, size/version-guarded service tables, `HostSlot` for later use. |
| `unit` | `ff13_unit!`, the `Init` scope (init-only services), host return codes, panic containment. |
| `sites` | `sites!` tables and the shared verification policy (below). |
| `hooks` | All-or-nothing hook sets with rollback; enable/disable/remove later. |
| `storage` | Host storage regions in the mod's `mod-state.dat` (never the game's saves). |
| `packed` | The packed record format and `#[derive(PackedRecord)]`. |
| `settings` | `settings!` providers for the shared Mod Settings menu. |
| `save_events` | Save / load / new-game notifications. |
| `param` | `Param`: a lock-free whole-number setting for hook paths (clamped, change callback); bind it to the menu with `Builder::param_*`. |
| `per_save` | `PerSave`: settings and small state kept per save slot in host storage, keyed by the save events' profile and slot. |
| `texts` | The XIII menu's ZTR keys for a unit's own texts; catalog and container checks. |
| `log` | Allocation-free stack lines for hook paths. |
| `pe` | Export-table checks. |
| `testing` | `FakeHost`: an in-memory host for unit tests. |
| `win` | (Windows) live memory queries, code writes, module pinning. |

### Site policy

Verify every address before touching anything. What a unit does with a
site decides what is accepted:

* `call` — the unit only calls the function: the retail bytes, or another
  component's entry detour (`E9 rel32` or `FF 25 [abs32]` at the entry,
  every later verified byte retail, destination outside the main image in
  committed executable memory). Calls chain through the detour.
* `hook` — the unit hooks or redirects it: retail bytes only; another
  component's detour is reported as occupied and never overwritten.
* `anchor` — code or data read or patched by value: retail bytes only.

Anything else is a different executable: fail closed and leave the game
untouched.

### Packed records

Byte 0 is the format version; numeric fields follow LSB-first in frozen
order, each `value - min` in `ceil(log2(max - min + 1))` bits; an optional
text tail of `(section, key, value)` entries follows. Never reorder or
resize released fields: bump `schema` and migrate.

```rust
#[derive(workshop_ext_sdk::PackedRecord)]
#[packed(version = 1, schema = 2, migrate = from_schema_1)]
struct Options {
    #[packed(default = 1, max = 11)]
    destination: u8,
    verbose_log: bool,
    #[packed(text)]
    text: Vec<workshop_ext_sdk::packed::TextEntry>,
}
```

### Per-save settings

`per_save::PerSave` keeps a unit's settings with the player's save slot,
in one host storage region (`<unit-id>.per_save`); the game's saves are
never read or written. `per_save::Params` adapts a set of `Param`s (one of
them the per-slot *Enabled*); implement `per_save::Adapter` for other state.

| Moment | What happens |
| --- | --- |
| process start | defaults, everything off |
| menu edits | memory only |
| `SAVE_BEGIN` | the state is captured per operation id (at most 64 pending) |
| `SAVED` | that capture is written (revision-checked batch, one retry when the header is unchanged) and flushed |
| `SAVE_FAILED` | the capture is discarded |
| `LOADED` | queued writes finish, then the slot's record is restored; no record: defaults, off |
| `NEW_GAME` | memory is kept |
| invalid slot/profile | logged; a load restores defaults, off; never slot 0 |

The region is a 32-byte header (`FF13SLOT`, container 1, layout hash, slot
count, record size, profile capacity 16, profiles in use, flags, CRC-32), a
16 × 32-byte profile directory and fixed records at
`544 + (profile × slots + slot) × record`. A record is a flag byte (valid,
Enabled, version), the payload and a CRC-32 over the profile, slot, flags
and payload. A region of another layout is never read or written.
`PerSave::import` seeds a region never written once (e.g. from a legacy
file), in batches of at most 64 KiB.

```rust
use workshop_ext_sdk::{param::Param, per_save::{Params, PerSave}};
static ENABLED: Param = Param::flag("Enabled", false);
static LEVEL: Param = Param::new("Level", 3, 1, 9);
static PARAMS: Params = Params::new(1, &ENABLED, &[&LEVEL]);
static STORE: PerSave = PerSave::new("example.unit", &PARAMS);
// init: STORE.open(init.storage(&STORE.region_id().unwrap())?, slot_count);
//       save-event handler: |event| STORE.on_event(event)
```

## Host contract

The C headers of the Crux host (`ff13_host.h` and the settings and
save-event headers it includes) are the contract; this crate mirrors them
and fails its tests on drift (`src/header.rs`). Fields are only ever
appended: check `version` and use `size` (which `Host` does for you).

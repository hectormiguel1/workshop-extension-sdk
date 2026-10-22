# workshop-extension-sdk

The Rust SDK for **Crux extensions**: native DLL mods for the PC (Steam)
releases of FINAL FANTASY XIII, XIII-2 and LIGHTNING RETURNS.

A Crux extension (an *extension unit*) is a 32-bit Windows DLL that ships
inside a mod's CrystalWorkshop Shard (`.cws`). Crux loads it into the game,
calls its `init` export once and keeps it resident. The DLL exports exactly
`init` and `shutdown`, nothing else.

`workshop-ext-sdk` gives a unit:

- the host ABI as `#[repr(C)]` types, tested against Crux's C headers;
- `ff13_unit!` for the two exports, with panics caught at the boundary;
- `sites!` tables of byte-verified game addresses and one call/hook policy;
- all-or-nothing hook installs with rollback;
- host storage regions and `#[derive(PackedRecord)]` records;
- `settings!` pages for the shared in-game Mod Settings menu;
- save events and `PerSave`, for state that follows the player's save
  slot (mods never write the game's saves);
- allocation-free logging into `Crux.jsonl`;
- `FakeHost`, an in-memory host for tests.

## Quick start

```sh
rustup target add i686-pc-windows-gnu   # plus a MinGW-w64 i686 linker
cargo new --lib my-unit
```

`Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
workshop-ext-sdk = { git = "https://github.com/hectormiguel1/workshop-extension-sdk", tag = "v0.1.0" }
```

`src/lib.rs`:

```rust
use workshop_ext_sdk::prelude::*;

const NEEDS: Requirements = Requirements {
    game: Some(abi::GAME_XIII),
    services: &[Service::Log],
    image: true,
};

fn start(init: &Init) -> InitResult {
    let host = init.host();
    host.require(&NEEDS).map_err(|why| {
        host.error(format_args!("my unit: {why}"));
        InitError::UNUSABLE_CONTEXT
    })?;
    host.info(format_args!("my unit: running on game {}", host.game()));
    Ok(())
}

workshop_ext_sdk::ff13_unit! { init: start }
```

```sh
cargo build --release --target i686-pc-windows-gnu
```

Then add `target/i686-pc-windows-gnu/release/my_unit.dll` to a
CrystalWorkshop project, export a `.cws` shard and install it with
PulseDepot.

## Documentation

- [Overview](docs/README.md)
- [Set up a project](docs/setup.md)
- [Units and exports](docs/units.md)
- [Sites and hooks](docs/sites-and-hooks.md)
- [In-game settings](docs/settings.md)
- [Storage and per-save state](docs/per-save.md)
- [Logging](docs/logging.md)
- [Package a shard](docs/packaging.md)
- [Testing](docs/testing.md)
- [Walkthrough: hello-unit](docs/hello-unit.md)
- [API reference](docs/reference.md) (and `cargo doc -p workshop-ext-sdk --open`)

The same guide is on the
[CrystalWorkshop site](https://hectormiguel1.github.io/crystalworkshop-site/sdk/).

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/workshop-ext-sdk` | the SDK |
| `crates/workshop-ext-sdk/include` | Crux's C headers (the host contract the SDK is tested against) |
| `crates/workshop-ext-sdk-derive` | `#[derive(PackedRecord)]` |
| `examples/hello-unit` | a complete example unit with tests |
| `docs` | the guide |

```sh
cargo test --workspace                                   # host tests
cargo build -p hello-unit --target i686-pc-windows-gnu --profile host-dll
cargo run -p workshop-ext-sdk --example export_check -- \
    target/i686-pc-windows-gnu/host-dll/hello_unit.dll
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

FINAL FANTASY, FINAL FANTASY XIII, FINAL FANTASY XIII-2 and LIGHTNING
RETURNS: FINAL FANTASY XIII are trademarks or registered trademarks of
Square Enix. This is an unofficial fan project, not affiliated with,
endorsed by or sponsored by Square Enix. It contains no game data.

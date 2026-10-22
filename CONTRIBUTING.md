# Contributing

Thanks for helping. Bug reports, fixes, docs and examples are welcome.

## Before you open a pull request

- Open an issue first for anything larger than a small fix, so we can
  agree on the approach.
- Run the same checks as CI:

  ```sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo build -p hello-unit --target i686-pc-windows-gnu --profile host-dll
  ```

- Add or update tests for what you change. The Cucumber suite lives in
  `crates/workshop-ext-sdk/tests/`
  (`cargo test -p workshop-ext-sdk --test features`).
- Keep the ABI mirror in step with the headers in
  `crates/workshop-ext-sdk/include/`: the host contract only ever appends
  fields, and the tests fail on drift.
- Note user-visible changes in `CHANGELOG.md` under "Unreleased".

## Rules of the SDK

- No runtime dependencies besides the derive crate.
- Panics never cross the FFI boundary.
- Units never write the game's save files; their state goes to host
  storage.

## License

By contributing you agree that your contributions are dual licensed under
MIT or Apache-2.0, as described in the README.

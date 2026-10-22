# Changelog

All notable changes to this project are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-21

First public release.

### Added

- `workshop-ext-sdk`: the Crux host ABI (`abi`), the validated host
  context (`host`), the `init`/`shutdown` exports (`ff13_unit!`, `unit`),
  byte-verified sites with the call/hook/anchor policy (`sites!`,
  `sites`), all-or-nothing hook installs (`hooks`), host storage regions
  (`storage`), packed records (`packed`), Mod Settings providers
  (`settings!`, `settings`), lock-free settings values (`param`), save
  events (`save_events`), per-save state (`per_save`), menu texts
  (`texts`), allocation-free log lines (`log`), export-table checks
  (`pe`), an in-memory test host (`testing`) and Windows helpers (`win`).
- `workshop-ext-sdk-derive`: `#[derive(PackedRecord)]`.
- `examples/hello-unit`: a complete example unit with tests.
- `export_check`: verifies that built DLLs export exactly `init` and
  `shutdown`.
- The guide in `docs/`.

[Unreleased]: https://github.com/hectormiguel1/workshop-extension-sdk/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hectormiguel1/workshop-extension-sdk/releases/tag/v0.1.0

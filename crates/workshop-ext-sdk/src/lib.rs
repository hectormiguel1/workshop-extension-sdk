//! # workshop-ext-sdk
//!
//! The SDK for **Crux extension units**: native DLL mods for the
//! FINAL FANTASY XIII trilogy (32-bit Windows; Wine/Proton) that the
//! Crux host loads from a mod's container and initializes with its
//! service context.
//!
//! A unit is an ordinary `cdylib` that exports exactly `init` and
//! `shutdown`. This crate gives it:
//!
//! * [`abi`] — the host ABI, defined once (`#[repr(C)]` mirrors of the C
//!   headers, checked against them by this crate's tests);
//! * [`host`] — a validated [`host::Host`]: logging, merged-container file
//!   reads, version/size-guarded service tables;
//! * [`unit`](mod@unit) — [`ff13_unit!`] for the two exports, the init scope
//!   ([`unit::Init`]) and the host's return codes; panics never cross the
//!   boundary;
//! * [`sites`](mod@sites) — [`sites!`] tables of byte-verified game addresses with one
//!   shared policy for retail bytes and other mods' detours;
//! * [`hooks`] — create/enable/disable/remove through the host's MinHook,
//!   all-or-nothing installs with rollback;
//! * [`storage`] — host-owned persistent regions (`mod-state.dat`; never
//!   the game's saves) and [`packed`] records for them, with
//!   `#[derive(PackedRecord)]`;
//! * [`settings`](mod@settings) — [`settings!`] providers for the shared Mod Settings menu;
//! * [`save_events`] — save/load/new-game notifications;
//! * [`param`] — lock-free settings values for hook paths;
//! * [`per_save`] — settings and state kept per save slot (host storage
//!   keyed by the save events' profile and slot; never the game's saves);
//! * [`texts`] — a unit's own menu texts (ZTR keys, catalog checks);
//! * [`log`] — allocation-free stack lines for hook paths;
//! * [`pe`] — export-table checks for built units;
//! * [`testing`] — an in-memory host for unit tests.
//!
//! ```no_run
//! use workshop_ext_sdk::{abi, host::{Requirements, Service}, unit::{Init, InitError, InitResult}};
//!
//! const NEEDS: Requirements = Requirements {
//!     game: Some(abi::GAME_XIII),
//!     services: &[Service::Log],
//!     image: true,
//! };
//!
//! fn start(init: &Init) -> InitResult {
//!     let host = init.host();
//!     host.require(&NEEDS).map_err(|why| {
//!         host.error(format_args!("hello: {why}"));
//!         InitError::UNUSABLE_CONTEXT
//!     })?;
//!     host.info(format_args!("hello: running on game {}", host.game()));
//!     Ok(())
//! }
//!
//! workshop_ext_sdk::ff13_unit! { init: start }
//! ```
//!
//! Build the shipping DLL for `i686-pc-windows-gnu` with a profile that
//! keeps `panic = "unwind"` (the boundary catches panics) and verify its
//! exports with the `export_check` example. See the crate README and
//! `examples/hello-unit` for a complete unit with settings, storage and a
//! verified call site.
#![warn(missing_docs)]

// Lets `#[derive(PackedRecord)]` expand inside this crate's own tests.
extern crate self as workshop_ext_sdk;

pub mod abi;
pub mod hooks;
pub mod host;
pub mod image;
pub mod log;
pub mod packed;
pub mod param;
pub mod pe;
pub mod per_save;
pub mod save_events;
pub mod settings;
pub mod sites;
pub mod storage;
pub mod testing;
pub mod texts;
pub mod unit;

/// Windows-only helpers (live process memory, code writes, pinning).
#[cfg(windows)]
pub mod win;

#[cfg(test)]
mod header;

/// `#[derive(PackedRecord)]`: see [`packed::PackedRecord`].
#[cfg(feature = "derive")]
pub use workshop_ext_sdk_derive::PackedRecord;

/// Common imports for units.
pub mod prelude {
    pub use crate::abi::{self, level};
    pub use crate::host::{Host, HostSlot, Requirements, Service, Unusable};
    pub use crate::image::Image;
    pub use crate::packed::{PackedRecord, TextEntry};
    pub use crate::param::Param;
    pub use crate::per_save::{Params, PerSave};
    pub use crate::sites::{Site, SiteKind};
    pub use crate::unit::{Init, InitError, InitResult};
}

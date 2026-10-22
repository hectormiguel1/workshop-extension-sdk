//! A minimal Crux extension unit for FINAL FANTASY XIII (Steam
//! `ffxiiiimg.exe`) built on `workshop-ext-sdk`:
//!
//! * one verified **call site** (`BattleChara_GetHp`, which the unit could
//!   call; another mod's entry detour there is accepted);
//! * a **storage record** counting the sessions the unit ran in, kept by
//!   the host in the mod's `mod-state.dat` (never in the game's saves);
//! * a **settings provider** with a section and a checkbox in the shared
//!   Mod Settings menu.
//!
//! Everything below the exports is plain, host-testable Rust.
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use workshop_ext_sdk::prelude::*;
use workshop_ext_sdk::sites::{self, Finding, Memory};

/// Stable unit, storage region and settings provider ID.
pub const ID: &str = "example.hello";

/// What the unit needs from the host.
pub const NEEDS: Requirements = Requirements {
    game: Some(abi::GAME_XIII),
    services: &[Service::Log],
    image: true,
};

workshop_ext_sdk::sites! {
    /// The sites the unit relies on (bytes from the verified Steam build).
    pub const SITES = {
        "BattleChara_GetHp" @ 0x0006_e2d0 = b"\x55\x8b\xec\x51\x89\x4d\xfc\xb9\x47\x00\x00\x00\x83\xe1\x07\xb8", call;
    }
}

/// The persisted record (schema 1).
#[derive(Clone, Debug, PartialEq, workshop_ext_sdk::PackedRecord)]
#[packed(version = 1, schema = 1)]
pub struct Saved {
    /// Sessions the unit initialized in (saturating).
    #[packed(max = 65535)]
    pub sessions: u16,
    /// The menu checkbox.
    #[packed(default = 1)]
    pub greet: bool,
}

static GREET: AtomicBool = AtomicBool::new(true);
static SESSIONS: AtomicU32 = AtomicU32::new(0);
static HOST: HostSlot = HostSlot::new();

workshop_ext_sdk::settings! {
    /// The unit's Mod Settings page.
    pub mod menu {
        id: ID,
        name: "Hello unit",
        author: "workshop-ext-sdk",
        release: env!("CARGO_PKG_VERSION"),
        help: "The SDK's example unit.",
        game: abi::GAME_XIII,
        controls: |b| b
            .section("general", "General", "")
            .checkbox("greet", "Greet at field idle", "Log a greeting when the unit starts.",
                || GREET.load(Ordering::Relaxed),
                |on| GREET.store(on, Ordering::Relaxed)),
    }
}

/// The unit's init, given a reader of the loaded image and the process
/// memory the site policy consults.
pub fn start(
    init: &Init,
    read: &dyn Fn(u32, &mut [u8]) -> bool,
    memory: &dyn Memory,
) -> InitResult {
    let host = init.host();
    host.require(&NEEDS).map_err(|why| {
        host.error(format_args!("hello: {why}"));
        InitError::UNUSABLE_CONTEXT
    })?;
    let summary =
        sites::verify(
            SITES,
            host.image(),
            read,
            memory,
            &mut |site, finding| match finding {
                Finding::Problem(p) => host.error(format_args!(
                    "hello: {} @ RVA {:#x}: {p}",
                    site.name, site.rva
                )),
                Finding::Foreign { jump, dest } => host.info(format_args!(
                    "hello: {} @ RVA {:#x}: foreign {} -> {dest:#x} accepted",
                    site.name,
                    site.rva,
                    jump.text()
                )),
            },
        );
    if !summary.passed() {
        return Err(InitError::REFUSED);
    }
    let mut saved = Saved::defaults();
    if let Some(region) = init.storage(ID).map_err(|why| {
        host.error(format_args!("hello: {why}"));
        InitError::REFUSED
    })? {
        let (revision, stored) = match region.read_all() {
            Ok(Some((info, bytes))) => (info.revision, Saved::load(info.schema, &bytes)),
            Ok(None) => (0, None),
            Err(rc) => {
                host.warn(format_args!("hello: storage unreadable ({rc})"));
                (0, None)
            }
        };
        saved = stored.unwrap_or(saved);
        saved.sessions = saved.sessions.saturating_add(1);
        let bytes = saved.encode().ok_or(InitError::REFUSED)?;
        if let Err(rc) = region.write(revision, Saved::LAYOUT.schema, &bytes) {
            host.warn(format_args!("hello: storage write failed ({rc})"));
        }
    }
    GREET.store(saved.greet, Ordering::Relaxed);
    SESSIONS.store(saved.sessions.into(), Ordering::Relaxed);
    if let Some(settings) = init.settings()? {
        settings
            .register(menu::ID, menu::query)
            .map_err(|_| InitError::REFUSED)?;
    }
    HOST.set(host);
    if GREET.load(Ordering::Relaxed) {
        host.info(format_args!("hello: session {}", saved.sessions));
    }
    Ok(())
}

/// Sessions counted at init.
pub fn sessions() -> u32 {
    SESSIONS.load(Ordering::Relaxed)
}

fn live_start(init: &Init) -> InitResult {
    let image = init.host().image();
    // SAFETY: the host's image range is the mapped main executable.
    let read = |rva: u32, out: &mut [u8]| live_read(image, rva, out);
    start(init, &read, &live_memory())
}

#[cfg(windows)]
fn live_read(image: Image, rva: u32, out: &mut [u8]) -> bool {
    // SAFETY: the host's image range is the mapped main executable.
    unsafe { image.read(rva, out) }
}
#[cfg(windows)]
fn live_memory() -> workshop_ext_sdk::win::LiveMemory {
    workshop_ext_sdk::win::LiveMemory
}
#[cfg(not(windows))]
fn live_read(_: Image, _: u32, _: &mut [u8]) -> bool {
    false
}
#[cfg(not(windows))]
fn live_memory() -> sites::NoForeignCode {
    sites::NoForeignCode
}

fn stop() {
    if let Some(host) = HOST.get() {
        host.info(format_args!("hello: shutdown"));
    }
}

workshop_ext_sdk::ff13_unit! { init: live_start, shutdown: stop }

#[cfg(test)]
mod tests {
    use super::*;
    use workshop_ext_sdk::{testing::FakeHost, unit::run_init};

    fn image(rva: u32, out: &mut [u8]) -> bool {
        let ok = rva == SITES[0].rva && out.len() == SITES[0].bytes.len();
        if ok {
            out.copy_from_slice(SITES[0].bytes);
        }
        ok
    }

    #[test]
    fn counts_sessions_and_registers_its_menu() {
        let host = FakeHost::new(abi::GAME_XIII);
        let run =
            || unsafe { run_init(host.context(), |i| start(i, &image, &sites::NoForeignCode)) };
        assert_eq!(run(), 0);
        assert_eq!(sessions(), 1);
        assert!(FakeHost::logged("hello: session 1"));
        let menu = FakeHost::query(ID).unwrap();
        assert_eq!(menu.count, 2);
        // Next session: the host still holds the record.
        let region = FakeHost::take_region(ID).unwrap();
        assert_eq!(
            Saved::decode(&region.bytes),
            Some(Saved {
                sessions: 1,
                greet: true
            })
        );
        let host = FakeHost::new(abi::GAME_XIII);
        FakeHost::seed_region(ID, region.schema, &region.bytes);
        assert_eq!(
            unsafe { run_init(host.context(), |i| start(i, &image, &sites::NoForeignCode)) },
            0
        );
        assert_eq!(sessions(), 2);
        stop();
        assert!(FakeHost::logged("hello: shutdown"));
    }

    #[test]
    fn refuses_other_games_and_builds() {
        let host = FakeHost::new(abi::GAME_XIII_2);
        assert_eq!(
            unsafe { run_init(host.context(), |i| start(i, &image, &sites::NoForeignCode)) },
            1
        );
        assert!(FakeHost::logged("game 2 is not FINAL FANTASY XIII"));
        let host = FakeHost::new(abi::GAME_XIII);
        assert_eq!(
            unsafe {
                run_init(host.context(), |i| {
                    start(i, &|_, _| false, &sites::NoForeignCode)
                })
            },
            2
        );
        assert!(FakeHost::logged("outside the loaded image"));
        assert_eq!(unsafe { init(core::ptr::null()) }, 1);
        shutdown();
    }
}

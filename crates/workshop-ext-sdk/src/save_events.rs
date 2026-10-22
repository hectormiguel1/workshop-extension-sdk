//! Save lifecycle events: the host tells a unit when the game saves, loads
//! or starts a new game, so mod state in host storage can follow the
//! player's slot. Mods never write the game's save files.
//!
//! Register in `init` ([`crate::unit::Init::save_events`]). Callbacks run
//! synchronously on the game's operation thread: keep them short, never
//! start a nested save/load, and persist only after [`EventKind::Saved`].
//! A missing slot/profile identity must never be treated as slot 0.
use crate::abi::{self, SaveEvent, SaveEventInfo, SaveEventsApi, status};

/// The kind of a [`SaveEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A save starts: snapshot state.
    SaveBegin,
    /// The save completed: persist the snapshot.
    Saved,
    /// The save failed: drop the snapshot.
    SaveFailed,
    /// Accepted game state was loaded.
    Loaded,
    /// A new game started (no slot): reset transient state.
    NewGame,
}

impl EventKind {
    /// Every kind, in ABI order.
    pub const ALL: [EventKind; 5] = [
        EventKind::SaveBegin,
        EventKind::Saved,
        EventKind::SaveFailed,
        EventKind::Loaded,
        EventKind::NewGame,
    ];

    /// The ABI value (`FF13_SAVE_BEGIN` ..).
    pub const fn raw(self) -> u32 {
        match self {
            EventKind::SaveBegin => abi::SAVE_BEGIN,
            EventKind::Saved => abi::SAVED,
            EventKind::SaveFailed => abi::SAVE_FAILED,
            EventKind::Loaded => abi::LOADED,
            EventKind::NewGame => abi::NEW_GAME,
        }
    }

    /// The kind of an ABI value.
    pub const fn from_raw(kind: u32) -> Option<EventKind> {
        match kind {
            abi::SAVE_BEGIN => Some(EventKind::SaveBegin),
            abi::SAVED => Some(EventKind::Saved),
            abi::SAVE_FAILED => Some(EventKind::SaveFailed),
            abi::LOADED => Some(EventKind::Loaded),
            abi::NEW_GAME => Some(EventKind::NewGame),
            _ => None,
        }
    }

    /// `FF13_SAVE_EVENT_MASK(kind)`.
    pub const fn mask(self) -> u32 {
        abi::save_event_mask(self.raw())
    }
}

/// The mask of `kinds`.
pub const fn mask_of(kinds: &[EventKind]) -> u32 {
    let mut out = 0;
    let mut i = 0;
    while i < kinds.len() {
        out |= kinds[i].mask();
        i += 1;
    }
    out
}

impl SaveEvent {
    /// The event kind, if known.
    pub const fn event_kind(&self) -> Option<EventKind> {
        EventKind::from_raw(self.kind)
    }

    /// The slot, only when the host flagged it valid.
    pub const fn valid_slot(&self) -> Option<u32> {
        if self.flags & abi::SAVE_SLOT_VALID != 0 && self.slot != abi::SAVE_NO_SLOT {
            Some(self.slot)
        } else {
            None
        }
    }

    /// The profile identity, only when the host flagged it valid.
    pub const fn valid_profile(&self) -> Option<&[u8; 32]> {
        if self.flags & abi::SAVE_PROFILE_VALID != 0 {
            Some(&self.profile)
        } else {
            None
        }
    }
}

/// A unit's save-event handler: return 0, or nonzero to have it logged
/// (results are advisory; they never veto a retail save or load).
pub type Handler = fn(&SaveEvent) -> i32;

/// Status returned to the host when a handler panicked (the panic is
/// caught: it must never unwind into the game).
pub const HANDLER_PANICKED: i32 = -1000;

unsafe extern "C" fn trampoline(event: *const SaveEvent, user: usize) -> i32 {
    // SAFETY: host contract: `event` is null or valid for the call.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return status::INVALID_ARGUMENT;
    };
    if (event.size as usize) < size_of::<SaveEvent>() {
        return status::INVALID_ARGUMENT;
    }
    // SAFETY: `user` is the `Handler` that `register` passed.
    let handler = unsafe { core::mem::transmute::<usize, Handler>(user) };
    std::panic::catch_unwind(|| handler(event)).unwrap_or(HANDLER_PANICKED)
}

/// The save events service of an init scope.
#[derive(Clone, Copy, Debug)]
pub struct SaveEvents {
    api: SaveEventsApi,
}

impl SaveEvents {
    pub(crate) const fn new(api: &SaveEventsApi) -> Self {
        Self { api: *api }
    }

    /// The service of a raw table. Prefer [`crate::unit::Init::save_events`],
    /// which validates the table and enforces the init scope.
    ///
    /// # Safety
    /// Register only on the thread running the unit's `init` (host
    /// contract), with a table the host advertised.
    pub const unsafe fn from_raw(api: &SaveEventsApi) -> Self {
        Self { api: *api }
    }

    /// What the host's adapter supports for the running executable.
    pub fn info(&self) -> Result<SaveEventInfo, i32> {
        let info = self.api.info.ok_or(status::UNAVAILABLE)?;
        let mut out = SaveEventInfo {
            size: size_of::<SaveEventInfo>() as u32,
            ..SaveEventInfo::default()
        };
        // SAFETY: host contract; `out` is writable.
        match unsafe { info(&mut out) } {
            status::OK => Ok(out),
            rc => Err(rc),
        }
    }

    /// Register `handler` for the event kinds in `mask` under the stable
    /// `id`. `UNAVAILABLE`: no adapter for this executable; `HOOK_FAILED`:
    /// the adapter's hooks conflict.
    pub fn register(&self, id: &str, mask: u32, handler: Handler) -> Result<(), i32> {
        if mask == 0 || mask & !abi::SAVE_ALL_EVENTS != 0 || id.is_empty() {
            return Err(status::INVALID_ARGUMENT);
        }
        let register = self.api.register_callback.ok_or(status::UNAVAILABLE)?;
        // SAFETY: host contract; the trampoline and `handler` (a fn item)
        // stay valid for the module lifetime.
        match unsafe {
            register(
                id.as_ptr(),
                id.len(),
                mask,
                Some(trampoline),
                handler as usize,
            )
        } {
            status::OK => Ok(()),
            rc => Err(rc),
        }
    }
}

#[cfg(test)]
#[path = "save_events_tests.rs"]
mod tests;

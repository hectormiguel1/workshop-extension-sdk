//! Safe access to the host context: validation ([`Requirements`]), logging,
//! merged-container reads, and the version/size-guarded service tables.
//!
//! [`Host`] is a validated copy of the context the host passed to `init`.
//! The host keeps its context and callbacks valid for the process lifetime,
//! so a `Host` may be stored (see [`HostSlot`]) and used from any thread.
use crate::abi::{self, HostContext, HostContextV1, level, status};
use crate::hooks::HookControl;
use crate::image::Image;
use crate::log::Line;
use core::fmt;
use core::sync::atomic::{AtomicPtr, Ordering};

/// A host service a unit can require.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// `log`.
    Log,
    /// `vfs_size`.
    VfsSize,
    /// `vfs_read`.
    VfsRead,
    /// `hook_create`.
    HookCreate,
    /// `hook_enable`.
    HookEnable,
    /// `hook_disable`.
    HookDisable,
    /// `hook_remove`.
    HookRemove,
}

impl Service {
    /// The four MinHook services, in creation/rollback order.
    pub const HOOKS: [Service; 4] = [
        Service::HookCreate,
        Service::HookEnable,
        Service::HookDisable,
        Service::HookRemove,
    ];

    /// The context field name (used in [`Unusable::MissingService`]).
    pub const fn name(self) -> &'static str {
        match self {
            Service::Log => "log",
            Service::VfsSize => "vfs_size",
            Service::VfsRead => "vfs_read",
            Service::HookCreate => "hook_create",
            Service::HookEnable => "hook_enable",
            Service::HookDisable => "hook_disable",
            Service::HookRemove => "hook_remove",
        }
    }

    fn present(self, c: &HostContextV1) -> bool {
        match self {
            Service::Log => c.log.is_some(),
            Service::VfsSize => c.vfs_size.is_some(),
            Service::VfsRead => c.vfs_read.is_some(),
            Service::HookCreate => c.hook_create.is_some(),
            Service::HookEnable => c.hook_enable.is_some(),
            Service::HookDisable => c.hook_disable.is_some(),
            Service::HookRemove => c.hook_remove.is_some(),
        }
    }
}

/// What a unit needs from a context. Checked in this order: ABI version,
/// prefix size, game, each service in the listed order, mapped image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    /// The game the unit was verified against, if any.
    pub game: Option<u32>,
    /// Services that must be present (the first missing one is reported).
    pub services: &'static [Service],
    /// Require a nonzero `game_base` and `game_size`.
    pub image: bool,
}

impl Requirements {
    /// Only the ABI version and prefix size.
    pub const ANY: Requirements = Requirements {
        game: None,
        services: &[],
        image: false,
    };
}

/// Why a context cannot drive a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// `init(NULL)`.
    Null,
    /// Another ABI version: no other field is read.
    Version(u32),
    /// Smaller than the version 1 prefix: no field past `size` is read.
    Size(u32),
    /// Another game than the unit was verified against.
    Game {
        /// `HostContext::game`.
        found: u32,
        /// [`Requirements::game`].
        required: u32,
    },
    /// A required service pointer is null.
    MissingService(&'static str),
    /// `game_base` or `game_size` is zero.
    Image,
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("host context is null"),
            Self::Version(v) => write!(f, "host ABI version {v} (need {})", abi::HOST_ABI_VERSION),
            Self::Size(s) => write!(f, "host context size {s} is too small"),
            Self::Game { found, required } => {
                write!(f, "game {found} is not {}", abi::game_name(*required))
            }
            Self::MissingService(name) => write!(f, "host service {name} is missing"),
            Self::Image => f.write_str("host context has no game image"),
        }
    }
}

/// Why a merged-container file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// A host status from `vfs_size` / `vfs_read` (e.g. `NOT_FOUND`).
    Status(i32),
    /// The file is larger than the caller's limit.
    TooLarge(u64),
    /// A service is absent.
    Unavailable,
}

/// A malformed optional service table: the host advertised it, but its
/// version, size or entries are not usable. Units should refuse to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// `FF13StateApi` version or size.
    StorageVersion,
    /// `FF13StateApi` lacks a required entry.
    StorageIncomplete,
    /// `FF13SettingsApi` version, size or `register_provider`.
    SettingsVersion,
    /// `FF13SaveEventsApi` version, size or `register_callback`.
    SaveEventsVersion,
}

impl ServiceError {
    /// A short description (also the `Display` text).
    pub const fn text(self) -> &'static str {
        match self {
            Self::StorageVersion => "storage service version/size",
            Self::StorageIncomplete => "storage service table incomplete",
            Self::SettingsVersion => "settings service version/size",
            Self::SaveEventsVersion => "save events service version/size",
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

impl HostContextV1 {
    /// Checks `required` (see [`Requirements`] for the order).
    pub fn require(&self, required: &Requirements) -> Result<(), Unusable> {
        if self.version != abi::HOST_ABI_VERSION {
            return Err(Unusable::Version(self.version));
        }
        if (self.size as usize) < abi::prefix_size() {
            return Err(Unusable::Size(self.size));
        }
        if let Some(game) = required.game
            && self.game != game
        {
            return Err(Unusable::Game {
                found: self.game,
                required: game,
            });
        }
        if let Some(missing) = required.services.iter().find(|s| !s.present(self)) {
            return Err(Unusable::MissingService(missing.name()));
        }
        if required.image && (self.game_base == 0 || self.game_size == 0) {
            return Err(Unusable::Image);
        }
        Ok(())
    }

    /// Log `line` through the host (no-op without the service).
    pub fn log_line(&self, level: u32, line: &[u8]) {
        if let Some(log) = self.log {
            // SAFETY: host contract: `line` is readable for its length and
            // the host does not retain it.
            unsafe { log(level, line.as_ptr(), line.len()) };
        }
    }

    /// Decoded size of a merged-container file.
    pub fn file_size(&self, path: &str) -> Result<u64, ReadError> {
        let size_of_file = self.vfs_size.ok_or(ReadError::Unavailable)?;
        let mut size = 0u64;
        // SAFETY: host contract; `path` and `size` are valid for the call.
        match unsafe { size_of_file(path.as_ptr(), path.len(), &mut size) } {
            status::OK => Ok(size),
            rc => Err(ReadError::Status(rc)),
        }
    }

    /// Read a whole merged-container file, refusing files above `max` bytes.
    pub fn read_file(&self, path: &str, max: u64) -> Result<Vec<u8>, ReadError> {
        let (true, Some(read)) = (self.vfs_size.is_some(), self.vfs_read) else {
            return Err(ReadError::Unavailable);
        };
        let size = self.file_size(path)?;
        if size > max {
            return Err(ReadError::TooLarge(size));
        }
        let mut out = vec![0u8; size as usize];
        let mut written = 0usize;
        // SAFETY: `out` holds `size` writable bytes.
        let rc = unsafe {
            read(
                path.as_ptr(),
                path.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut written,
            )
        };
        if rc != status::OK {
            return Err(ReadError::Status(rc));
        }
        out.truncate(written);
        Ok(out)
    }
}

/// A validated host context (ABI version 1, at least the prefix).
///
/// Cheap to copy. Services the host did not advertise read as absent.
#[derive(Clone, Copy, Debug)]
pub struct Host {
    raw: *const HostContext,
    context: HostContext,
}

// SAFETY: the host keeps the context and its callbacks valid for the
// process lifetime and documents every service as callable from any thread
// (except the init-only registrations, which only `Init` exposes).
unsafe impl Send for Host {}
// SAFETY: as above; `Host` is immutable.
unsafe impl Sync for Host {}

impl Host {
    /// Validate and copy the context at `ptr`: null, another ABI version and
    /// a context smaller than the prefix are refused before any other field
    /// is read.
    ///
    /// # Safety
    /// `ptr` is null or a host context readable for its advertised `size`
    /// that stays valid for the process lifetime.
    pub unsafe fn from_raw(ptr: *const HostContext) -> Result<Host, Unusable> {
        if ptr.is_null() {
            return Err(Unusable::Null);
        }
        // SAFETY: caller contract; every version starts with version, size.
        let (version, size) = unsafe {
            let words = ptr as *const u32;
            (words.read_unaligned(), words.add(1).read_unaligned())
        };
        if version != abi::HOST_ABI_VERSION {
            return Err(Unusable::Version(version));
        }
        if (size as usize) < abi::prefix_size() {
            return Err(Unusable::Size(size));
        }
        // SAFETY: caller contract; `read` copies at most `size` bytes.
        let context = unsafe { HostContext::read(ptr) }.ok_or(Unusable::Null)?;
        Ok(Host { raw: ptr, context })
    }

    /// The copied context (tail fields `None` when `size` does not cover them).
    pub const fn context(&self) -> &HostContext {
        &self.context
    }

    /// The host's own pointer (what `init` received).
    pub const fn raw(&self) -> *const HostContext {
        self.raw
    }

    /// The version 1 prefix.
    pub const fn prefix(&self) -> HostContextV1 {
        self.context.prefix()
    }

    /// `HostContext::game`.
    pub const fn game(&self) -> u32 {
        self.context.game
    }

    /// `HostContext::phase`.
    pub const fn phase(&self) -> u32 {
        self.context.phase
    }

    /// The loaded main image.
    pub const fn image(&self) -> Image {
        Image {
            base: self.context.game_base,
            size: self.context.game_size,
        }
    }

    /// Checks `required` (see [`Requirements`]).
    pub fn require(&self, required: &Requirements) -> Result<(), Unusable> {
        self.prefix().require(required)
    }

    /// Log raw bytes (UTF-8 expected; the host logs at most 4096 bytes).
    pub fn log_bytes(&self, level: u32, line: &[u8]) {
        self.prefix().log_line(level, line);
    }

    /// Log a string.
    pub fn log(&self, level: u32, line: &str) {
        self.log_bytes(level, line.as_bytes());
    }

    /// Format `args` into a 512-byte stack line and log it (no allocation).
    pub fn log_fmt(&self, level: u32, args: fmt::Arguments<'_>) {
        let line: Line<512> = crate::log::line(args);
        self.log_bytes(level, line.as_bytes());
    }

    /// Log at [`level::ERROR`].
    pub fn error(&self, args: fmt::Arguments<'_>) {
        self.log_fmt(level::ERROR, args);
    }

    /// Log at [`level::WARN`].
    pub fn warn(&self, args: fmt::Arguments<'_>) {
        self.log_fmt(level::WARN, args);
    }

    /// Log at [`level::INFO`].
    pub fn info(&self, args: fmt::Arguments<'_>) {
        self.log_fmt(level::INFO, args);
    }

    /// Decoded size of a merged-container file.
    pub fn file_size(&self, path: &str) -> Result<u64, ReadError> {
        self.prefix().file_size(path)
    }

    /// Read a whole merged-container file (at most `max` bytes).
    pub fn read_file(&self, path: &str, max: u64) -> Result<Vec<u8>, ReadError> {
        self.prefix().read_file(path, max)
    }

    /// Enable/disable/remove for hooks this unit created (all three present).
    pub fn hooks(&self) -> Option<HookControl> {
        HookControl::from_context(&self.context)
    }

    /// The storage table, when advertised: `Err` when malformed.
    pub fn storage_api(&self) -> Result<Option<&'static abi::StateApi>, ServiceError> {
        let end = core::mem::offset_of!(HostContext, settings);
        let Some(api) = self.context.storage.filter(|_| self.context.covers(end)) else {
            return Ok(None);
        };
        if api.version != abi::STATE_VERSION || (api.size as usize) < size_of::<abi::StateApi>() {
            return Err(ServiceError::StorageVersion);
        }
        if api.register_region.is_none()
            || api.info.is_none()
            || api.read_at.is_none()
            || api.write_batch.is_none()
        {
            return Err(ServiceError::StorageIncomplete);
        }
        Ok(Some(api))
    }

    /// The settings registry, when advertised: `Err` when malformed.
    pub fn settings_api(&self) -> Result<Option<&'static abi::SettingsApi>, ServiceError> {
        let end = core::mem::offset_of!(HostContext, save_events);
        let Some(api) = self.context.settings.filter(|_| self.context.covers(end)) else {
            return Ok(None);
        };
        if api.version != abi::SETTINGS_VERSION
            || (api.size as usize) < size_of::<abi::SettingsApi>()
            || api.register_provider.is_none()
        {
            return Err(ServiceError::SettingsVersion);
        }
        Ok(Some(api))
    }

    /// The save events service, when advertised: `Err` when malformed.
    pub fn save_events_api(&self) -> Result<Option<&'static abi::SaveEventsApi>, ServiceError> {
        let end = size_of::<HostContext>();
        let Some(api) = self
            .context
            .save_events
            .filter(|_| self.context.covers(end))
        else {
            return Ok(None);
        };
        if api.version != abi::SAVE_EVENTS_VERSION
            || (api.size as usize) < size_of::<abi::SaveEventsApi>()
            || api.register_callback.is_none()
        {
            return Err(ServiceError::SaveEventsVersion);
        }
        Ok(Some(api))
    }
}

/// A process-wide slot for the host context a unit was initialized with,
/// for hooks and `shutdown` that run after `init` returned.
pub struct HostSlot(AtomicPtr<HostContext>);

impl HostSlot {
    /// An empty slot.
    pub const fn new() -> Self {
        Self(AtomicPtr::new(core::ptr::null_mut()))
    }

    /// Remember `host`'s context.
    pub fn set(&self, host: &Host) {
        self.0.store(host.raw.cast_mut(), Ordering::Release);
    }

    /// The remembered context (re-read from the host's pointer).
    pub fn get(&self) -> Option<Host> {
        let ptr = self.0.load(Ordering::Acquire);
        // SAFETY: only `set` stores here, with a context validated by
        // `Host::from_raw` that the host keeps valid for the process.
        unsafe { Host::from_raw(ptr) }.ok()
    }

    /// Forget the context.
    pub fn clear(&self) {
        self.0.store(core::ptr::null_mut(), Ordering::Release);
    }
}

impl Default for HostSlot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;

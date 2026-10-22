//! The raw host ABI: `#[repr(C)]` mirrors of `ff13_host.h`,
//! `ff13_settings.h` and `ff13_save_events.h` (host ABI version 1), with
//! their constants and callback types.
//!
//! Everything here is plain data. The host only ever *appends* fields: a
//! unit checks `version` and uses `size` to tell which trailing fields exist.
//! [`HostContext::read`] copies only the advertised bytes, so a context from
//! an older host reads its missing tail as absent. Prefer the safe wrappers
//! in [`crate::host`] over touching these structs directly.
//!
//! The layouts are the 32-bit (x86) ones the game uses; the same Rust types
//! build for 64-bit hosts so unit logic can be tested there. The crate's
//! tests parse the C headers and fail when they drift from these mirrors.
use core::ffi::{c_char, c_void};

/// `FF13_HOST_ABI_VERSION`: the only context version this SDK accepts.
pub const HOST_ABI_VERSION: u32 = 1;

/// `FF13_PHASE_EARLY`: init while the merged mod image loads, before the title screen.
pub const PHASE_EARLY: u32 = 0;
/// `FF13_PHASE_LATE`: init at the first field idle of the session.
pub const PHASE_LATE: u32 = 1;

/// `HostContext::game` for FINAL FANTASY XIII.
pub const GAME_XIII: u32 = 1;
/// `HostContext::game` for FINAL FANTASY XIII-2.
pub const GAME_XIII_2: u32 = 2;
/// `HostContext::game` for LIGHTNING RETURNS: FINAL FANTASY XIII.
pub const GAME_LR: u32 = 3;

/// Display name of a `HostContext::game` value (`"unknown game"` otherwise).
pub const fn game_name(game: u32) -> &'static str {
    match game {
        GAME_XIII => "FINAL FANTASY XIII",
        GAME_XIII_2 => "FINAL FANTASY XIII-2",
        GAME_LR => "LIGHTNING RETURNS: FINAL FANTASY XIII",
        _ => "unknown game",
    }
}

/// Service status codes (`FF13_OK`, ...). Negative values are errors.
pub mod status {
    /// `FF13_OK`.
    pub const OK: i32 = 0;
    /// `FF13_INVALID_ARGUMENT`.
    pub const INVALID_ARGUMENT: i32 = -1;
    /// `FF13_NOT_FOUND` (also: a storage region that was never written).
    pub const NOT_FOUND: i32 = -2;
    /// `FF13_SHORT_BUFFER`: `*written` receives the required size.
    pub const SHORT_BUFFER: i32 = -3;
    /// `FF13_DECODE_FAILED`.
    pub const DECODE_FAILED: i32 = -4;
    /// `FF13_UNAVAILABLE`.
    pub const UNAVAILABLE: i32 = -5;
    /// `FF13_HOOK_FAILED`.
    pub const HOOK_FAILED: i32 = -6;
    /// `FF13_STATE_IO_ERROR`: the batch may have been committed.
    pub const STATE_IO_ERROR: i32 = -7;
    /// `FF13_STATE_CONFLICT`: another component owns the ID, or a stale revision.
    pub const STATE_CONFLICT: i32 = -8;
    /// `FF13_STATE_LIMIT`.
    pub const STATE_LIMIT: i32 = -9;
    /// `FF13_STATE_RECOVERY_REQUIRED`: storage is poisoned until next launch.
    pub const STATE_RECOVERY_REQUIRED: i32 = -10;
}

/// Log levels for [`HostContext::log`] (`FF13_LOG_*`).
pub mod level {
    /// `FF13_LOG_ERROR`.
    pub const ERROR: u32 = 0;
    /// `FF13_LOG_WARN`.
    pub const WARN: u32 = 1;
    /// `FF13_LOG_INFO`.
    pub const INFO: u32 = 2;
    /// `FF13_LOG_DEBUG`.
    pub const DEBUG: u32 = 3;
}

/// The host logs at most this many bytes of one message.
pub const LOG_MESSAGE_MAX: usize = 4096;

/// `FF13LogFn`: UTF-8 `message` of `length` bytes, not NUL-terminated.
pub type LogFn = unsafe extern "C" fn(level: u32, message: *const u8, length: usize);
/// `FF13VfsSizeFn`: decoded size of a merged-container path.
pub type VfsSizeFn = unsafe extern "C" fn(path: *const u8, length: usize, size: *mut u64) -> i32;
/// `FF13VfsReadFn`: the whole decoded file.
pub type VfsReadFn = unsafe extern "C" fn(
    path: *const u8,
    length: usize,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32;
/// `FF13HookCreateFn`: a disabled MinHook hook; the trampoline goes to `*original`.
pub type HookCreateFn = unsafe extern "C" fn(
    target: *mut c_void,
    detour: *mut c_void,
    original: *mut *mut c_void,
) -> i32;
/// `FF13HookTargetFn`: enable, disable or remove a hook this unit created.
pub type HookTargetFn = unsafe extern "C" fn(target: *mut c_void) -> i32;
/// `FF13InitFn`: the `init` export.
pub type InitFn = unsafe extern "C" fn(context: *const HostContext) -> i32;
/// `FF13ShutdownFn`: the `shutdown` export.
pub type ShutdownFn = unsafe extern "C" fn();

/// The ABI version 1 prefix of `FF13HostContext` (through `hook_remove`):
/// 52 bytes on x86. Every version 1 host provides at least this much.
///
/// Use it when a unit needs only the prefix services; [`HostContext`] adds
/// the optional appended service tables.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HostContextV1 {
    /// [`HOST_ABI_VERSION`].
    pub version: u32,
    /// `sizeof(FF13HostContext)` as built by the host.
    pub size: u32,
    /// [`GAME_XIII`], [`GAME_XIII_2`] or [`GAME_LR`].
    pub game: u32,
    /// [`PHASE_EARLY`] or [`PHASE_LATE`].
    pub phase: u32,
    /// Base address of the loaded main image.
    pub game_base: usize,
    /// `SizeOfImage` of the main image.
    pub game_size: usize,
    /// Host log.
    pub log: Option<LogFn>,
    /// Merged-container file size.
    pub vfs_size: Option<VfsSizeFn>,
    /// Merged-container file read.
    pub vfs_read: Option<VfsReadFn>,
    /// Hook creation (init thread only).
    pub hook_create: Option<HookCreateFn>,
    /// Hook enable.
    pub hook_enable: Option<HookTargetFn>,
    /// Hook disable.
    pub hook_disable: Option<HookTargetFn>,
    /// Hook removal.
    pub hook_remove: Option<HookTargetFn>,
}

/// `FF13HostContext` as the current host builds it (64 bytes on x86).
///
/// The trailing service tables exist only when `size` covers them; read a
/// context from the host with [`HostContext::read`], which never copies
/// past `size`, and reach the tables through [`crate::host::Host`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HostContext {
    /// [`HOST_ABI_VERSION`].
    pub version: u32,
    /// `sizeof(FF13HostContext)` as built by the host.
    pub size: u32,
    /// [`GAME_XIII`], [`GAME_XIII_2`] or [`GAME_LR`].
    pub game: u32,
    /// [`PHASE_EARLY`] or [`PHASE_LATE`].
    pub phase: u32,
    /// Base address of the loaded main image.
    pub game_base: usize,
    /// `SizeOfImage` of the main image.
    pub game_size: usize,
    /// Host log.
    pub log: Option<LogFn>,
    /// Merged-container file size.
    pub vfs_size: Option<VfsSizeFn>,
    /// Merged-container file read.
    pub vfs_read: Option<VfsReadFn>,
    /// Hook creation (init thread only).
    pub hook_create: Option<HookCreateFn>,
    /// Hook enable.
    pub hook_enable: Option<HookTargetFn>,
    /// Hook disable.
    pub hook_disable: Option<HookTargetFn>,
    /// Hook removal.
    pub hook_remove: Option<HookTargetFn>,
    /// Host storage (`size` must cover offset 56 on x86).
    pub storage: Option<&'static StateApi>,
    /// Settings registry (`size` must cover offset 60 on x86).
    pub settings: Option<&'static SettingsApi>,
    /// Save lifecycle events (`size` must cover offset 64 on x86).
    pub save_events: Option<&'static SaveEventsApi>,
}

/// Byte size of [`HostContextV1`] on x86.
pub const PREFIX_SIZE_X86: u32 = 52;
/// Byte size of [`HostContext`] on x86.
pub const CONTEXT_SIZE_X86: u32 = 64;

/// Field offsets of [`HostContextV1`] in declaration order for a
/// `pointer`-byte target; `prefix_offsets(4)` is the x86 layout.
pub const fn prefix_offsets(pointer: usize) -> [usize; 13] {
    let all = context_offsets(pointer);
    let mut out = [0; 13];
    let mut i = 0;
    while i < 13 {
        out[i] = all[i];
        i += 1;
    }
    out
}

/// Field offsets of [`HostContext`] in declaration order for a
/// `pointer`-byte target; `context_offsets(4)` is the x86 layout.
pub const fn context_offsets(pointer: usize) -> [usize; 16] {
    let mut out = [0; 16];
    let mut i = 0;
    while i < 4 {
        out[i] = i * 4;
        i += 1;
    }
    let mut at = 16usize.next_multiple_of(pointer);
    while i < 16 {
        out[i] = at;
        at += pointer;
        i += 1;
    }
    out
}

/// Byte size of the [`HostContextV1`] prefix on this target.
pub const fn prefix_size() -> usize {
    core::mem::offset_of!(HostContext, storage)
}

impl HostContext {
    /// Copies the context the host advertises: at most `size` bytes, so an
    /// older (shorter) context reads its missing tail as absent (`None`).
    ///
    /// # Safety
    /// `ptr` is null, or a host context readable for its `size` field
    /// (and at least for the 8-byte `version`/`size` header).
    pub unsafe fn read(ptr: *const HostContext) -> Option<HostContext> {
        if ptr.is_null() {
            return None;
        }
        // SAFETY: caller contract; `size` is at offset 4 of every version.
        let size = unsafe { (ptr as *const u32).add(1).read_unaligned() } as usize;
        let n = size.min(size_of::<HostContext>());
        let mut out = core::mem::MaybeUninit::<HostContext>::zeroed();
        // SAFETY: `n` bytes are readable; every field is valid when zeroed
        // (integers, `Option` of fn pointers/references) or host-written.
        unsafe {
            core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr() as *mut u8, n);
            Some(out.assume_init())
        }
    }

    /// Whether `size` covers the field bytes up to `end`.
    pub const fn covers(&self, end: usize) -> bool {
        self.size as usize >= end
    }

    /// The version 1 prefix.
    pub const fn prefix(&self) -> HostContextV1 {
        HostContextV1 {
            version: self.version,
            size: self.size,
            game: self.game,
            phase: self.phase,
            game_base: self.game_base,
            game_size: self.game_size,
            log: self.log,
            vfs_size: self.vfs_size,
            vfs_read: self.vfs_read,
            hook_create: self.hook_create,
            hook_enable: self.hook_enable,
            hook_disable: self.hook_disable,
            hook_remove: self.hook_remove,
        }
    }

    /// A context holding `prefix` and no appended service tables.
    pub const fn from_prefix(prefix: HostContextV1) -> HostContext {
        HostContext {
            version: prefix.version,
            size: prefix.size,
            game: prefix.game,
            phase: prefix.phase,
            game_base: prefix.game_base,
            game_size: prefix.game_size,
            log: prefix.log,
            vfs_size: prefix.vfs_size,
            vfs_read: prefix.vfs_read,
            hook_create: prefix.hook_create,
            hook_enable: prefix.hook_enable,
            hook_disable: prefix.hook_disable,
            hook_remove: prefix.hook_remove,
            storage: None,
            settings: None,
            save_events: None,
        }
    }
}

// ---- storage (`FF13StateApi`) ----------------------------------------------

/// `FF13_STATE_VERSION`.
pub const STATE_VERSION: u32 = 1;
/// Region limit per mod root.
pub const STATE_MAX_REGIONS: usize = 256;
/// Bytes per region.
pub const STATE_MAX_REGION_BYTES: u64 = 4 << 20;
/// Ranges per batch or read.
pub const STATE_MAX_RANGES: usize = 256;
/// Input bytes per batch or read.
pub const STATE_MAX_BATCH_BYTES: usize = 64 << 10;
/// Longest region ID (ASCII letters, digits, `.`, `_`, `-`).
pub const STATE_MAX_ID: usize = 64;

/// `FF13StateInfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateInfo {
    /// `sizeof(FF13StateInfo)`; set it before passing an output buffer.
    pub size: u32,
    /// Schema of the stored bytes (> 0 once written).
    pub schema: u32,
    /// Logical length.
    pub length: u64,
    /// Revision for the next `write_batch`.
    pub revision: u64,
    /// Compression hint (0 or 1, retained v1 client hints).
    pub compression: u32,
    /// Reserved, 0.
    pub reserved: u32,
}

/// `FF13StateRange`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StateRange {
    /// Byte offset in the region.
    pub offset: u64,
    /// The bytes to write.
    pub data: *const u8,
    /// Length of `data`.
    pub length: usize,
}

/// `register_region(id, length, handle)`.
pub type RegisterRegionFn =
    unsafe extern "C" fn(id: *const u8, length: usize, handle: *mut u32) -> i32;
/// `info(handle, output)`.
pub type StateInfoFn = unsafe extern "C" fn(handle: u32, output: *mut StateInfo) -> i32;
/// `read_at(handle, offset, output, length)`.
pub type ReadAtFn =
    unsafe extern "C" fn(handle: u32, offset: u64, output: *mut u8, length: usize) -> i32;
/// `write_batch(handle, expected_revision, new_length, schema, compression, ranges, count, output)`.
pub type WriteBatchFn = unsafe extern "C" fn(
    handle: u32,
    expected_revision: u64,
    new_length: u64,
    schema: u32,
    compression: u32,
    ranges: *const StateRange,
    count: usize,
    output: *mut StateInfo,
) -> i32;
/// `flush()`.
pub type FlushFn = unsafe extern "C" fn() -> i32;

/// `FF13StateApi` (version 1).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StateApi {
    /// [`STATE_VERSION`].
    pub version: u32,
    /// `sizeof(FF13StateApi)`.
    pub size: u32,
    /// Init only.
    pub register_region: Option<RegisterRegionFn>,
    /// Region metadata.
    pub info: Option<StateInfoFn>,
    /// Ranged read.
    pub read_at: Option<ReadAtFn>,
    /// Atomic, durable batch.
    pub write_batch: Option<WriteBatchFn>,
    /// Flush (optional).
    pub flush: Option<FlushFn>,
}

// ---- settings (`ff13_settings.h`) ------------------------------------------

/// `FF13_SETTINGS_VERSION`.
pub const SETTINGS_VERSION: u32 = 1;
/// `FF13_SETTING_CHECKBOX`: value 0 or 1.
pub const SETTING_CHECKBOX: u32 = 1;
/// `FF13_SETTING_INTEGER`: whole numbers in `min..=max` by `step`.
pub const SETTING_INTEGER: u32 = 2;
/// `FF13_SETTING_FLOAT`.
pub const SETTING_FLOAT: u32 = 3;
/// `FF13_SETTING_BUTTON`: `activate` only.
pub const SETTING_BUTTON: u32 = 4;
/// `FF13_SETTING_COMBO`: option index `0..option_count`.
pub const SETTING_COMBO: u32 = 5;
/// `FF13_SETTING_SECTION`: starts a settings page/group.
pub const SETTING_SECTION: u32 = 11;
/// `FF13_SETTING_EXT_STATE`.
pub const SETTING_EXT_STATE: u32 = 0x1001;
/// Text extension of the host's Rust contract (`settings_contract.rs`).
pub const SETTING_EXT_TEXT: u32 = 0x1002;
/// `FF13_SETTING_EXT_PRESENTATION`.
pub const SETTING_EXT_PRESENTATION: u32 = 0x1004;
/// `FF13_SETTING_EXT_DISPLAY`.
pub const SETTING_EXT_DISPLAY: u32 = 0x1005;
/// State bit `FF13_SETTING_VISIBLE`.
pub const SETTING_VISIBLE: u32 = 1;
/// State bit `FF13_SETTING_ENABLED`.
pub const SETTING_ENABLED: u32 = 2;
/// State bit `FF13_SETTING_BUSY` (disables input).
pub const SETTING_BUSY: u32 = 8;
/// Presentation flag `FF13_SETTING_ADVANCED`.
pub const SETTING_ADVANCED: u32 = 1;
/// Presentation flag `FF13_SETTING_RESTART_REQUIRED`.
pub const SETTING_RESTART_REQUIRED: u32 = 2;
/// Presentation flag `FF13_SETTING_EXPERIMENTAL`.
pub const SETTING_EXPERIMENTAL: u32 = 4;
/// Controls per provider.
pub const SETTINGS_MAX_CONTROLS: usize = 256;
/// Bytes per ID (provider, section, control).
pub const SETTINGS_MAX_ID: usize = 64;
/// Bytes of a provider name.
pub const SETTINGS_MAX_NAME: usize = 128;
/// Bytes of a label.
pub const SETTINGS_MAX_LABEL: usize = 192;
/// Bytes of a help text.
pub const SETTINGS_MAX_HELP: usize = 1024;
/// Bytes of a display prefix/suffix.
pub const SETTINGS_MAX_AFFIX: usize = 24;
/// Extension records per chain.
pub const SETTINGS_MAX_EXTENSIONS: usize = 16;

/// `FF13SettingsQueryContext`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingsQueryContext {
    /// `sizeof(FF13SettingsQueryContext)`.
    pub size: u32,
    /// [`SETTINGS_VERSION`].
    pub version: u32,
    /// The running game.
    pub game: u32,
    /// NUL-terminated game name.
    pub game_name: *const c_char,
    /// NUL-terminated backend version.
    pub backend_version: *const c_char,
}

/// `FF13SettingExtension`: header of every control extension.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingExtension {
    /// `sizeof` the complete extension.
    pub size: u32,
    /// `SETTING_EXT_*`.
    pub kind: u32,
    /// 1.
    pub version: u32,
    /// Next extension or null.
    pub next: *const SettingExtension,
}

/// `read_state(user) -> SETTING_VISIBLE | SETTING_ENABLED | SETTING_BUSY`.
pub type ReadStateFn = unsafe extern "C" fn(user: *mut c_void) -> u32;
/// `read(user) -> value`.
pub type ReadValueFn = unsafe extern "C" fn(user: *mut c_void) -> f32;
/// `write(user, value)`.
pub type WriteValueFn = unsafe extern "C" fn(user: *mut c_void, value: f32);
/// `activate(user)`.
pub type ActivateFn = unsafe extern "C" fn(user: *mut c_void);
/// `read_text(user, output, capacity) -> length` (text extension).
pub type ReadTextFn =
    unsafe extern "C" fn(user: *mut c_void, output: *mut u8, capacity: u32) -> u32;

/// `FF13SettingStateExtension`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingStateExtension {
    /// `kind` = [`SETTING_EXT_STATE`].
    pub header: SettingExtension,
    /// Called when the control is displayed or used.
    pub read_state: Option<ReadStateFn>,
    /// Passed to `read_state`.
    pub user: *mut c_void,
}

/// The text extension of `settings_contract.rs` (`kind` = [`SETTING_EXT_TEXT`]).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingTextExtension {
    /// `kind` = [`SETTING_EXT_TEXT`].
    pub header: SettingExtension,
    /// Copies UTF-8 into `output`, returns the full length.
    pub read_text: Option<ReadTextFn>,
    /// Passed to `read_text`.
    pub user: *mut c_void,
    /// Refresh period in milliseconds.
    pub refresh_ms: u32,
}

/// `FF13SettingPresentationExtension`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingPresentationExtension {
    /// `kind` = [`SETTING_EXT_PRESENTATION`].
    pub header: SettingExtension,
    /// `SETTING_ADVANCED | SETTING_RESTART_REQUIRED | SETTING_EXPERIMENTAL`.
    pub flags: u32,
}

/// `FF13SettingDisplayExtension`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingDisplayExtension {
    /// `kind` = [`SETTING_EXT_DISPLAY`].
    pub header: SettingExtension,
    /// ZTR key or UTF-8, at most 24 bytes.
    pub prefix: *const c_char,
    /// ZTR key or UTF-8, at most 24 bytes.
    pub suffix: *const c_char,
    /// 0..=6, for numeric controls.
    pub decimals: u32,
}

/// `FF13SettingControl`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingControl {
    /// `sizeof(FF13SettingControl)`.
    pub size: u32,
    /// `SETTING_*` kind.
    pub kind: u32,
    /// Stable ID (independent of label and row).
    pub id: *const c_char,
    /// ZTR key or UTF-8.
    pub label: *const c_char,
    /// ZTR key or UTF-8.
    pub help: *const c_char,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Step.
    pub step: f32,
    /// Value read.
    pub read: Option<ReadValueFn>,
    /// Value write.
    pub write: Option<WriteValueFn>,
    /// Passed to `read`, `write` and `activate`.
    pub user: *mut c_void,
    /// Button press.
    pub activate: Option<ActivateFn>,
    /// Combo option count.
    pub option_count: u32,
    /// Combo options (ZTR keys or UTF-8).
    pub options: *const *const c_char,
    /// Extension chain or null.
    pub extensions: *const SettingExtension,
}

/// `FF13SettingsProvider`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingsProvider {
    /// `sizeof(FF13SettingsProvider)`.
    pub size: u32,
    /// [`SETTINGS_VERSION`].
    pub version: u32,
    /// Provider ID (the registered ID).
    pub id: *const c_char,
    /// Display name.
    pub name: *const c_char,
    /// Control count.
    pub count: u32,
    /// `count` controls, `control_stride` bytes apart.
    pub controls: *const SettingControl,
    /// Reserved, 0.
    pub reserved_menu_count: u32,
    /// Reserved.
    pub reserved_menu_stride: u32,
    /// Reserved, null.
    pub reserved_menus: *const c_void,
    /// `sizeof(FF13SettingControl)`.
    pub control_stride: u32,
    /// Author.
    pub author: *const c_char,
    /// Release/version text.
    pub release: *const c_char,
    /// Help text.
    pub help: *const c_char,
    /// Website.
    pub website: *const c_char,
    /// Reserved.
    pub reserved_color: [f32; 4],
    /// Reserved.
    pub reserved_flags: u32,
}

/// `FF13SettingsQuery`: metadata only; no callbacks run during a query.
pub type SettingsQueryFn =
    unsafe extern "C" fn(context: *const SettingsQueryContext) -> *const SettingsProvider;
/// `FF13SettingsVisitor`.
pub type SettingsVisitorFn = unsafe extern "C" fn(
    id: *const u8,
    length: usize,
    module: usize,
    query: SettingsQueryFn,
    user: usize,
) -> i32;
/// `register_provider(id, length, query)` (init thread only).
pub type RegisterProviderFn =
    unsafe extern "C" fn(id: *const u8, length: usize, query: Option<SettingsQueryFn>) -> i32;
/// `visit_providers(visitor, user)`.
pub type VisitProvidersFn =
    unsafe extern "C" fn(visitor: Option<SettingsVisitorFn>, user: usize) -> i32;

/// `FF13SettingsApi` (version 1).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingsApi {
    /// [`SETTINGS_VERSION`].
    pub version: u32,
    /// `sizeof(FF13SettingsApi)`.
    pub size: u32,
    /// Init thread only.
    pub register_provider: Option<RegisterProviderFn>,
    /// Provider enumeration.
    pub visit_providers: Option<VisitProvidersFn>,
}

// ---- save events (`ff13_save_events.h`) ------------------------------------

/// `FF13_SAVE_EVENTS_VERSION`.
pub const SAVE_EVENTS_VERSION: u32 = 1;
/// `FF13_SAVE_BEGIN`: a save starts (snapshot state here).
pub const SAVE_BEGIN: u32 = 1;
/// `FF13_SAVED`: the save completed (persist here).
pub const SAVED: u32 = 2;
/// `FF13_SAVE_FAILED`.
pub const SAVE_FAILED: u32 = 3;
/// `FF13_LOADED`: accepted game state (not a slot preview).
pub const LOADED: u32 = 4;
/// `FF13_NEW_GAME`: reset transient state (no slot).
pub const NEW_GAME: u32 = 5;
/// `FF13_SAVE_ALL_EVENTS`.
pub const SAVE_ALL_EVENTS: u32 = 31;
/// `FF13_SAVE_SLOT_VALID`.
pub const SAVE_SLOT_VALID: u32 = 1;
/// `FF13_SAVE_PROFILE_VALID`.
pub const SAVE_PROFILE_VALID: u32 = 2;
/// `FF13_SAVE_NO_SLOT`.
pub const SAVE_NO_SLOT: u32 = u32::MAX;

/// `FF13_SAVE_EVENT_MASK(kind)`.
pub const fn save_event_mask(kind: u32) -> u32 {
    1u32 << (kind - 1)
}

/// `FF13SaveEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveEvent {
    /// [`SAVE_EVENTS_VERSION`].
    pub version: u32,
    /// `sizeof(FF13SaveEvent)`.
    pub size: u32,
    /// The running game.
    pub game: u32,
    /// Executable variant.
    pub variant: u32,
    /// [`SAVE_BEGIN`] ..= [`NEW_GAME`].
    pub kind: u32,
    /// Slot, valid only with [`SAVE_SLOT_VALID`].
    pub slot: u32,
    /// Token pairing SAVE_BEGIN with SAVED / SAVE_FAILED.
    pub operation: u64,
    /// [`SAVE_SLOT_VALID`] | [`SAVE_PROFILE_VALID`].
    pub flags: u32,
    /// Reserved.
    pub reserved: u32,
    /// Profile identity, valid only with [`SAVE_PROFILE_VALID`].
    pub profile: [u8; 32],
}

/// `FF13SaveEventInfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveEventInfo {
    /// `sizeof(FF13SaveEventInfo)`; set before the call.
    pub size: u32,
    /// The running game.
    pub game: u32,
    /// Executable variant.
    pub variant: u32,
    /// Mask of supported events.
    pub events: u32,
    /// Slot count.
    pub slot_count: u32,
    /// Reserved.
    pub reserved: u32,
}

/// `FF13SaveEventCallback`: advisory; nonzero is logged, never vetoes.
pub type SaveEventCallbackFn = unsafe extern "C" fn(event: *const SaveEvent, user: usize) -> i32;
/// `info(output)`.
pub type SaveEventInfoFn = unsafe extern "C" fn(output: *mut SaveEventInfo) -> i32;
/// `register_callback(id, length, events, callback, user)` (init only).
pub type RegisterCallbackFn = unsafe extern "C" fn(
    id: *const u8,
    length: usize,
    events: u32,
    callback: Option<SaveEventCallbackFn>,
    user: usize,
) -> i32;

/// `FF13SaveEventsApi` (version 1).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SaveEventsApi {
    /// [`SAVE_EVENTS_VERSION`].
    pub version: u32,
    /// `sizeof(FF13SaveEventsApi)`.
    pub size: u32,
    /// Adapter information.
    pub info: Option<SaveEventInfoFn>,
    /// Init only.
    pub register_callback: Option<RegisterCallbackFn>,
}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;

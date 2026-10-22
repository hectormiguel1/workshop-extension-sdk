//! An in-memory host for unit tests: a complete [`HostContext`] whose
//! services (log, merged files, MinHook, storage, settings, save events)
//! record into per-thread state. Run tests that use it on one thread at a
//! time per state (the default test harness gives every test its own
//! thread; Cucumber runners should run scenarios one at a time).
//!
//! ```
//! use workshop_ext_sdk::{abi, testing::FakeHost};
//! let host = FakeHost::new(abi::GAME_XIII);
//! FakeHost::add_file("db/x.wdb", b"bytes");
//! let h = unsafe { workshop_ext_sdk::host::Host::from_raw(host.context()) }.unwrap();
//! h.info(format_args!("hello"));
//! assert_eq!(h.read_file("db/x.wdb", 64).unwrap(), b"bytes");
//! assert!(FakeHost::logged("hello"));
//! ```
use crate::abi::{
    self, HostContext, SaveEvent, SaveEventInfo, SaveEventsApi, SettingsApi, SettingsProvider,
    SettingsQueryContext, SettingsQueryFn, StateApi, StateInfo, StateRange, status,
};
use core::ffi::c_void;
use std::cell::RefCell;

/// One hook the fake MinHook created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FakeHook {
    /// Target address.
    pub target: usize,
    /// Detour address.
    pub detour: usize,
    /// The trampoline handed back (`target ^ 0x5a5a_0000`, never 0).
    pub original: usize,
    /// Enabled.
    pub enabled: bool,
}

/// One storage region of the fake host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeRegion {
    /// Region ID.
    pub id: String,
    /// Schema (0 = never written).
    pub schema: u32,
    /// Revision.
    pub revision: u64,
    /// Bytes.
    pub bytes: Vec<u8>,
    /// Registered in this session.
    pub registered: bool,
}

/// A registered save-event callback.
#[derive(Clone, Copy, Debug)]
pub struct FakeSaveCallback {
    /// Mask of events.
    pub events: u32,
    /// Callback.
    pub callback: abi::SaveEventCallbackFn,
    /// User value.
    pub user: usize,
}

#[derive(Default)]
struct State {
    lines: Vec<(u32, String)>,
    files: Vec<(String, Vec<u8>)>,
    hooks: Vec<FakeHook>,
    fail_create_at: Option<usize>,
    fail_enable_at: Option<usize>,
    null_original: bool,
    disabled: usize,
    removed: usize,
    regions: Vec<FakeRegion>,
    register_status: i32,
    write_status: i32,
    info_size: Option<u32>,
    providers: Vec<(String, SettingsQueryFn)>,
    provider_status: i32,
    save_callbacks: Vec<(String, FakeSaveCallback)>,
    save_status: i32,
    flushes: usize,
    flush_status: i32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

unsafe fn bytes<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if n == 0 {
        return &[];
    }
    // SAFETY: caller contract (host ABI: readable for `n`).
    unsafe { core::slice::from_raw_parts(p, n) }
}

unsafe fn text(p: *const u8, n: usize) -> String {
    // SAFETY: as `bytes`.
    String::from_utf8_lossy(unsafe { bytes(p, n) }).into_owned()
}

unsafe extern "C" fn log(level: u32, message: *const u8, length: usize) {
    // SAFETY: host ABI.
    let line = unsafe { text(message, length) };
    with(|s| s.lines.push((level, line)));
}

unsafe extern "C" fn vfs_size(path: *const u8, length: usize, size: *mut u64) -> i32 {
    // SAFETY: host ABI.
    let path = unsafe { text(path, length) };
    match with(|s| s.files.iter().find(|f| f.0 == path).map(|f| f.1.len())) {
        Some(n) => {
            // SAFETY: host ABI: `size` is writable.
            unsafe { size.write(n as u64) };
            status::OK
        }
        None => status::NOT_FOUND,
    }
}

unsafe extern "C" fn vfs_read(
    path: *const u8,
    length: usize,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    // SAFETY: host ABI.
    let path = unsafe { text(path, length) };
    let Some(file) = with(|s| s.files.iter().find(|f| f.0 == path).map(|f| f.1.clone())) else {
        return status::NOT_FOUND;
    };
    // SAFETY: host ABI: `written` is writable, `output` for `capacity`.
    unsafe {
        written.write(file.len());
        if file.len() > capacity {
            return status::SHORT_BUFFER;
        }
        if !file.is_empty() {
            core::ptr::copy_nonoverlapping(file.as_ptr(), output, file.len());
        }
    }
    status::OK
}

unsafe extern "C" fn hook_create(
    target: *mut c_void,
    detour: *mut c_void,
    original: *mut *mut c_void,
) -> i32 {
    let target = target as usize;
    let created = with(|s| {
        if s.fail_create_at == Some(s.hooks.len()) {
            return Err(status::HOOK_FAILED);
        }
        let trampoline = if s.null_original {
            0
        } else {
            (target ^ 0x5a5a_0000) | 1
        };
        s.hooks.push(FakeHook {
            target,
            detour: detour as usize,
            original: trampoline,
            enabled: false,
        });
        Ok(trampoline)
    });
    match created {
        Ok(trampoline) => {
            // SAFETY: host ABI: `original` is writable.
            unsafe { original.write(trampoline as *mut c_void) };
            status::OK
        }
        Err(rc) => rc,
    }
}

unsafe extern "C" fn hook_enable(target: *mut c_void) -> i32 {
    with(
        |s| match s.hooks.iter().position(|h| h.target == target as usize) {
            Some(i) if s.fail_enable_at == Some(i) => status::HOOK_FAILED,
            Some(i) => {
                s.hooks[i].enabled = true;
                status::OK
            }
            None => status::NOT_FOUND,
        },
    )
}

unsafe extern "C" fn hook_disable(target: *mut c_void) -> i32 {
    with(
        |s| match s.hooks.iter_mut().find(|h| h.target == target as usize) {
            Some(h) => {
                h.enabled = false;
                s.disabled += 1;
                status::OK
            }
            None => status::NOT_FOUND,
        },
    )
}

unsafe extern "C" fn hook_remove(target: *mut c_void) -> i32 {
    with(|s| {
        let before = s.hooks.len();
        s.hooks.retain(|h| h.target != target as usize);
        if s.hooks.len() == before {
            return status::NOT_FOUND;
        }
        s.removed += 1;
        status::OK
    })
}

unsafe extern "C" fn register_region(id: *const u8, length: usize, handle: *mut u32) -> i32 {
    // SAFETY: host ABI.
    let id = unsafe { text(id, length) };
    let result = with(|s| {
        if s.register_status != status::OK {
            return Err(s.register_status);
        }
        if let Some(i) = s.regions.iter().position(|r| r.id == id) {
            if s.regions[i].registered {
                return Err(status::STATE_CONFLICT);
            }
            s.regions[i].registered = true;
            return Ok(i as u32 + 1);
        }
        s.regions.push(FakeRegion {
            id,
            schema: 0,
            revision: 0,
            bytes: Vec::new(),
            registered: true,
        });
        Ok(s.regions.len() as u32)
    });
    match result {
        Ok(h) => {
            // SAFETY: host ABI: `handle` is writable.
            unsafe { handle.write(h) };
            status::OK
        }
        Err(rc) => rc,
    }
}

fn region<R>(handle: u32, f: impl FnOnce(&mut FakeRegion) -> R) -> Option<R> {
    with(|s| s.regions.get_mut((handle as usize).checked_sub(1)?).map(f))
}

unsafe extern "C" fn state_info(handle: u32, output: *mut StateInfo) -> i32 {
    let size = with(|s| s.info_size).unwrap_or(size_of::<StateInfo>() as u32);
    let info = region(handle, |r| {
        (r.schema != 0).then_some(StateInfo {
            size,
            schema: r.schema,
            length: r.bytes.len() as u64,
            revision: r.revision,
            compression: 0,
            reserved: 0,
        })
    });
    match info {
        None => status::INVALID_ARGUMENT,
        Some(None) => status::NOT_FOUND,
        Some(Some(info)) => {
            // SAFETY: host ABI: `output` is writable.
            unsafe { output.write(info) };
            status::OK
        }
    }
}

unsafe extern "C" fn read_at(handle: u32, offset: u64, output: *mut u8, length: usize) -> i32 {
    let bytes = region(handle, |r| {
        let start = usize::try_from(offset).ok()?;
        r.bytes
            .get(start..start.checked_add(length)?)
            .map(<[u8]>::to_vec)
    });
    match bytes {
        Some(Some(b)) => {
            // SAFETY: host ABI: `output` is writable for `length`.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), output, b.len()) };
            status::OK
        }
        _ => status::INVALID_ARGUMENT,
    }
}

unsafe extern "C" fn write_batch(
    handle: u32,
    expected: u64,
    new_length: u64,
    schema: u32,
    _compression: u32,
    ranges: *const StateRange,
    count: usize,
    output: *mut StateInfo,
) -> i32 {
    let forced = with(|s| s.write_status);
    if forced != status::OK {
        return forced;
    }
    let ranges: Vec<(u64, Vec<u8>)> = (0..count)
        .map(|i| {
            // SAFETY: host ABI: `count` ranges, each readable.
            unsafe {
                let r = *ranges.add(i);
                (r.offset, bytes(r.data, r.length).to_vec())
            }
        })
        .collect();
    // Like the host: at most 64 KiB per batch, no overlapping ranges.
    if ranges.iter().map(|r| r.1.len()).sum::<usize>() > abi::STATE_MAX_BATCH_BYTES {
        return status::STATE_LIMIT;
    }
    let mut sorted: Vec<(u64, usize)> = ranges
        .iter()
        .filter(|r| !r.1.is_empty())
        .map(|r| (r.0, r.1.len()))
        .collect();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0].0 + w[0].1 as u64 > w[1].0) {
        return status::INVALID_ARGUMENT;
    }
    let result = region(handle, |r| {
        if schema == 0 {
            return Err(status::INVALID_ARGUMENT);
        }
        if expected != r.revision {
            return Err(status::STATE_CONFLICT);
        }
        let mut next = r.bytes.clone();
        next.resize(new_length as usize, 0);
        for (offset, data) in &ranges {
            let at = *offset as usize;
            let dst = next
                .get_mut(at..at + data.len())
                .ok_or(status::INVALID_ARGUMENT)?;
            dst.copy_from_slice(data);
        }
        r.bytes = next;
        r.schema = schema;
        r.revision += 1;
        Ok(StateInfo {
            size: size_of::<StateInfo>() as u32,
            schema,
            length: new_length,
            revision: r.revision,
            compression: 0,
            reserved: 0,
        })
    });
    match result {
        Some(Ok(info)) => {
            // SAFETY: host ABI: `output` is writable.
            unsafe { output.write(info) };
            status::OK
        }
        Some(Err(rc)) => rc,
        None => status::INVALID_ARGUMENT,
    }
}

unsafe extern "C" fn flush() -> i32 {
    with(|s| {
        s.flushes += 1;
        s.flush_status
    })
}

unsafe extern "C" fn register_provider(
    id: *const u8,
    length: usize,
    query: Option<SettingsQueryFn>,
) -> i32 {
    // SAFETY: host ABI.
    let id = unsafe { text(id, length) };
    with(|s| {
        if s.provider_status != status::OK {
            return s.provider_status;
        }
        let Some(query) = query else {
            return status::INVALID_ARGUMENT;
        };
        if s.providers.iter().any(|p| p.0 == id) {
            return status::STATE_CONFLICT;
        }
        s.providers.push((id, query));
        status::OK
    })
}

unsafe extern "C" fn save_info(output: *mut SaveEventInfo) -> i32 {
    // SAFETY: host ABI: `output` is writable.
    unsafe {
        output.write(SaveEventInfo {
            size: size_of::<SaveEventInfo>() as u32,
            game: abi::GAME_XIII,
            variant: 0,
            events: abi::SAVE_ALL_EVENTS,
            slot_count: 100,
            reserved: 0,
        })
    };
    status::OK
}

unsafe extern "C" fn register_callback(
    id: *const u8,
    length: usize,
    events: u32,
    callback: Option<abi::SaveEventCallbackFn>,
    user: usize,
) -> i32 {
    // SAFETY: host ABI.
    let id = unsafe { text(id, length) };
    with(|s| {
        if s.save_status != status::OK {
            return s.save_status;
        }
        match callback {
            Some(callback) => {
                s.save_callbacks.push((
                    id,
                    FakeSaveCallback {
                        events,
                        callback,
                        user,
                    },
                ));
                status::OK
            }
            None => status::INVALID_ARGUMENT,
        }
    })
}

/// The fake storage table.
pub static STATE_API: StateApi = StateApi {
    version: abi::STATE_VERSION,
    size: size_of::<StateApi>() as u32,
    register_region: Some(register_region),
    info: Some(state_info),
    read_at: Some(read_at),
    write_batch: Some(write_batch),
    flush: Some(flush),
};

/// The fake settings table.
pub static SETTINGS_API: SettingsApi = SettingsApi {
    version: abi::SETTINGS_VERSION,
    size: size_of::<SettingsApi>() as u32,
    register_provider: Some(register_provider),
    visit_providers: None,
};

/// The fake save events table.
pub static SAVE_EVENTS_API: SaveEventsApi = SaveEventsApi {
    version: abi::SAVE_EVENTS_VERSION,
    size: size_of::<SaveEventsApi>() as u32,
    info: Some(save_info),
    register_callback: Some(register_callback),
};

/// A complete fake host context (boxed, so its address is stable).
pub struct FakeHost {
    context: Box<HostContext>,
}

impl FakeHost {
    /// Fresh per-thread state and a context for `game` with every service,
    /// image `0x0040_0000 + 0x0251_9000`, phase LATE.
    pub fn new(game: u32) -> FakeHost {
        with(|s| *s = State::default());
        FakeHost {
            context: Box::new(HostContext {
                version: abi::HOST_ABI_VERSION,
                size: size_of::<HostContext>() as u32,
                game,
                phase: abi::PHASE_LATE,
                game_base: 0x40_0000,
                game_size: 0x251_9000,
                log: Some(log),
                vfs_size: Some(vfs_size),
                vfs_read: Some(vfs_read),
                hook_create: Some(hook_create),
                hook_enable: Some(hook_enable),
                hook_disable: Some(hook_disable),
                hook_remove: Some(hook_remove),
                storage: Some(&STATE_API),
                settings: Some(&SETTINGS_API),
                save_events: Some(&SAVE_EVENTS_API),
            }),
        }
    }

    /// The pointer `init` receives.
    pub fn context(&self) -> *const HostContext {
        &*self.context
    }

    /// The context, to break it for a test.
    pub fn context_mut(&mut self) -> &mut HostContext {
        &mut self.context
    }

    /// Every log line so far.
    pub fn lines() -> Vec<(u32, String)> {
        with(|s| s.lines.clone())
    }

    /// Whether any log line contains `text`.
    pub fn logged(text: &str) -> bool {
        with(|s| s.lines.iter().any(|l| l.1.contains(text)))
    }

    /// Serve `bytes` at the merged-container `path`.
    pub fn add_file(path: &str, bytes: &[u8]) {
        with(|s| s.files.push((path.into(), bytes.to_vec())));
    }

    /// Hooks currently created.
    pub fn hooks() -> Vec<FakeHook> {
        with(|s| s.hooks.clone())
    }

    /// Fail the `n`th hook creation (0-based) with `HOOK_FAILED`.
    pub fn fail_hook_create_at(n: usize) {
        with(|s| s.fail_create_at = Some(n));
    }

    /// Fail enabling the `n`th created hook with `HOOK_FAILED`.
    pub fn fail_hook_enable_at(n: usize) {
        with(|s| s.fail_enable_at = Some(n));
    }

    /// Hand back null trampolines (a broken MinHook).
    pub fn null_trampolines() {
        with(|s| s.null_original = true);
    }

    /// `(disables, removals)` so far.
    pub fn hook_cleanups() -> (usize, usize) {
        with(|s| (s.disabled, s.removed))
    }

    /// Storage regions.
    pub fn regions() -> Vec<FakeRegion> {
        with(|s| s.regions.clone())
    }

    /// Pre-seed a region as a previous session left it (registered later).
    pub fn seed_region(id: &str, schema: u32, bytes: &[u8]) {
        with(|s| {
            s.regions.push(FakeRegion {
                id: id.into(),
                schema,
                revision: 1,
                bytes: bytes.to_vec(),
                registered: false,
            })
        });
    }

    /// Remove a region (e.g. to carry it into the next session with
    /// [`FakeHost::seed_region`]).
    pub fn take_region(id: &str) -> Option<FakeRegion> {
        with(|s| {
            let i = s.regions.iter().position(|r| r.id == id)?;
            Some(s.regions.remove(i))
        })
    }

    /// Change a region in place (keeping its handle), e.g. to bump its
    /// revision as another writer would, or to truncate it.
    pub fn edit_region(id: &str, f: impl FnOnce(&mut FakeRegion)) {
        with(|s| {
            if let Some(r) = s.regions.iter_mut().find(|r| r.id == id) {
                f(r)
            }
        });
    }

    /// Make `register_region`, `write_batch`, `register_provider` and
    /// `register_callback` return these statuses (0 = normal).
    pub fn fail_services(register: i32, write: i32, provider: i32, save: i32) {
        with(|s| {
            s.register_status = register;
            s.write_status = write;
            s.provider_status = provider;
            s.save_status = save;
        });
    }

    /// Make `info` report this `StateInfo::size` (to test short tables).
    pub fn info_size(size: u32) {
        with(|s| s.info_size = Some(size));
    }

    /// Make `flush` return this status (0 = normal); it still counts.
    pub fn fail_flush(status: i32) {
        with(|s| s.flush_status = status);
    }

    /// Storage flushes so far.
    pub fn flushes() -> usize {
        with(|s| s.flushes)
    }

    /// Registered settings providers.
    pub fn providers() -> Vec<(String, SettingsQueryFn)> {
        with(|s| s.providers.clone())
    }

    /// Query the provider registered as `id` like the XIII menu adapter.
    pub fn query(id: &str) -> Option<&'static SettingsProvider> {
        let query = with(|s| s.providers.iter().find(|p| p.0 == id).map(|p| p.1))?;
        let context = SettingsQueryContext {
            size: size_of::<SettingsQueryContext>() as u32,
            version: abi::SETTINGS_VERSION,
            game: abi::GAME_XIII,
            game_name: c"FINAL FANTASY XIII".as_ptr(),
            backend_version: c"test".as_ptr(),
        };
        // SAFETY: a registered query and a complete context.
        unsafe { query(&context).as_ref() }
    }

    /// Registered save-event callbacks.
    pub fn save_callbacks() -> Vec<(String, FakeSaveCallback)> {
        with(|s| s.save_callbacks.clone())
    }

    /// Deliver `kind` (slot `slot`, valid when `Some`) to every subscribed
    /// callback; their results.
    pub fn fire(kind: u32, slot: Option<u32>) -> Vec<i32> {
        Self::deliver(&save_event(kind, slot))
    }

    /// Deliver `event` to every callback subscribed to its kind; their results.
    pub fn deliver(event: &SaveEvent) -> Vec<i32> {
        let kind = event.kind;
        let event = *event;
        Self::save_callbacks()
            .into_iter()
            .filter(|(_, c)| c.events & abi::save_event_mask(kind) != 0)
            // SAFETY: a registered callback and a complete event.
            .map(|(_, c)| unsafe { (c.callback)(&event, c.user) })
            .collect()
    }
}

/// A complete save event.
pub fn save_event(kind: u32, slot: Option<u32>) -> SaveEvent {
    SaveEvent {
        version: abi::SAVE_EVENTS_VERSION,
        size: size_of::<SaveEvent>() as u32,
        game: abi::GAME_XIII,
        variant: 0,
        kind,
        slot: slot.unwrap_or(abi::SAVE_NO_SLOT),
        operation: 7,
        flags: if slot.is_some() {
            abi::SAVE_SLOT_VALID
        } else {
            0
        },
        reserved: 0,
        profile: [0; 32],
    }
}

/// A complete save event with an explicit profile (valid when `Some`) and
/// operation id.
pub fn event(kind: u32, slot: Option<u32>, profile: Option<[u8; 32]>, operation: u64) -> SaveEvent {
    let mut e = save_event(kind, slot);
    e.operation = operation;
    if let Some(profile) = profile {
        e.profile = profile;
        e.flags |= abi::SAVE_PROFILE_VALID;
    }
    e
}

/// A fake [`crate::sites::Memory`]: executable ranges and readable dwords.
#[derive(Clone, Debug, Default)]
pub struct FakeMemory {
    /// `(start, len)` executable ranges.
    pub executable: Vec<(usize, usize)>,
    /// `(address, value)` readable dwords.
    pub dwords: Vec<(usize, u32)>,
}

impl crate::sites::Memory for FakeMemory {
    fn read_u32(&self, at: usize) -> Option<u32> {
        self.dwords.iter().find(|d| d.0 == at).map(|d| d.1)
    }
    fn executable(&self, at: usize) -> bool {
        self.executable.iter().any(|&(s, n)| at >= s && at - s < n)
    }
}

/// A minimal PE32 file whose export directory names `names` (for export
/// table tests); `magic` 0x10b is PE32, anything else is refused.
pub fn pe_image(names: &[&str], magic: u16) -> Vec<u8> {
    let mut b = vec![0u8; 0x400];
    let pe = 0x80;
    let mut put = |at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
    put(0x3c, &(pe as u32).to_le_bytes());
    put(pe, &0x4550u32.to_le_bytes());
    put(pe + 6, &1u16.to_le_bytes());
    put(pe + 20, &0xe0u16.to_le_bytes());
    put(pe + 24, &magic.to_le_bytes());
    let export_rva: u32 = if names.is_empty() { 0 } else { 0x1000 };
    put(pe + 24 + 96, &export_rva.to_le_bytes());
    let s = pe + 24 + 0xe0;
    put(s + 8, &0x200u32.to_le_bytes());
    put(s + 12, &0x1000u32.to_le_bytes());
    put(s + 16, &0x200u32.to_le_bytes());
    put(s + 20, &0x200u32.to_le_bytes());
    // Directory at file 0x200: count @24, name table RVA @32 (file 0x240).
    put(0x200 + 24, &(names.len() as u32).to_le_bytes());
    put(0x200 + 32, &0x1040u32.to_le_bytes());
    let mut at = 0x280;
    for (i, name) in names.iter().enumerate() {
        let rva = 0x1000 + (at - 0x200) as u32;
        put(0x240 + i * 4, &rva.to_le_bytes());
        put(at, name.as_bytes());
        at += name.len() + 1;
    }
    b
}

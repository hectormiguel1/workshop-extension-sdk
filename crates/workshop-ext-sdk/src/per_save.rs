//! Per-save state: settings (and other small state) that follow the
//! player's save slot instead of the installation.
//!
//! A unit keeps one host storage region, `<unit-id>.per_save`, in the
//! mod's `mod-state.dat`. Game saves are never read or written: the slot
//! identity comes from the host's save events ([`crate::save_events`]).
//!
//! # Lifecycle
//!
//! | Moment | What happens |
//! | --- | --- |
//! | process start | the adapter's defaults; everything **off** |
//! | menu edits | memory only (nothing is written) |
//! | `SAVE_BEGIN` | the adapter is captured, keyed by the operation id |
//! | `SAVED` | that capture is written (revision-checked batch) and flushed |
//! | `SAVE_FAILED` | the capture is discarded |
//! | `SAVED`/`SAVE_FAILED` of an unknown operation | ignored (logged) |
//! | `LOADED` | queued writes finish, then the slot's record is restored; no record: defaults, off |
//! | `NEW_GAME` | memory is kept (title-screen choices carry into the new game) |
//! | quit to title | nothing (no event): memory is unchanged |
//! | an event without a valid slot/profile | logged; a load restores defaults, off; never slot 0 |
//!
//! # Region layout (host storage schema [`REGION_SCHEMA`], little endian)
//!
//! ```text
//! 0    header (32 bytes)
//!        0  magic "FF13SLOT"
//!        8  u32 container version (CONTAINER)
//!       12  u32 layout hash (the adapter's; a different layout is not read)
//!       16  u16 slot count
//!       18  u16 record bytes
//!       20  u16 profile capacity (PROFILES)
//!       22  u16 profiles in use
//!       24  u32 flags (bit 0: imported from a legacy file)
//!       28  u32 CRC-32 of bytes 0..28
//! 32   profile directory: PROFILES × 32-byte profile ids (in use first)
//! 544  record blocks: profile i, slot s at 544 + (i × slots + s) × record bytes
//! ```
//!
//! A record is `flags` (bit 0 valid, bit 1 Enabled, bits 2..8 record
//! version), the adapter's fixed-size payload, then a CRC-32 over the
//! profile id, the slot (u16), the flags and the payload. Zero bytes (a
//! slot never saved) read as "no record".
//!
//! # Using it
//!
//! ```
//! use workshop_ext_sdk::param::Param;
//! use workshop_ext_sdk::per_save::{Params, PerSave};
//! static ENABLED: Param = Param::flag("Enabled", false);
//! static LEVEL: Param = Param::new("Level", 3, 1, 9);
//! static PARAMS: Params = Params::new(1, &ENABLED, &[&LEVEL]);
//! static STORE: PerSave = PerSave::new("example.unit", &PARAMS);
//! // in init: STORE.open(init.storage(&STORE.region_id().unwrap())?, slot_count);
//! //          register a save-event handler that calls STORE.on_event(event).
//! assert_eq!(STORE.region_id().unwrap(), "example.unit.per_save");
//! ```
use crate::abi::{SaveEvent, status};
use crate::packed::{self, Field, Layout};
use crate::param::Param;
use crate::save_events::EventKind;
use crate::storage::{self, Region};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};

/// Header magic.
pub const MAGIC: [u8; 8] = *b"FF13SLOT";
/// Header format version.
pub const CONTAINER: u32 = 1;
/// Host storage schema of the region.
pub const REGION_SCHEMA: u32 = 1;
/// Header bytes.
pub const HEADER_BYTES: usize = 32;
/// Bytes of one profile id.
pub const PROFILE_BYTES: usize = 32;
/// Profile directory capacity.
pub const PROFILES: usize = 16;
/// Where the record blocks start.
pub const RECORDS_AT: usize = HEADER_BYTES + PROFILES * PROFILE_BYTES;
/// Largest slot count a region accepts.
pub const MAX_SLOTS: u32 = 1000;
/// Largest adapter payload.
pub const MAX_PAYLOAD: usize = 1024;
/// SAVE_BEGIN captures kept at once.
pub const MAX_PENDING: usize = 64;
/// Record flag: the record holds a save.
pub const FLAG_VALID: u8 = 1;
/// Record flag: the per-save Enabled setting.
pub const FLAG_ENABLED: u8 = 2;
/// The record version sits in the flag bits from here up.
pub const VERSION_SHIFT: u32 = 2;
/// Largest record version.
pub const MAX_VERSION: u8 = 63;
/// Header flag: the region was seeded from a legacy file.
pub const HEADER_IMPORTED: u32 = 1;
/// Suffix of the region ID.
pub const SUFFIX: &str = ".per_save";

/// The region ID of `unit` (`None` when longer than 64 bytes or invalid).
pub fn region_id(unit: &str) -> Option<String> {
    let id = format!("{unit}{SUFFIX}");
    storage::valid_id(&id).then_some(id)
}

/// CRC-32 (IEEE) over `parts`.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for byte in parts.iter().flat_map(|p| p.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// FNV-1a (32 bit).
pub fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x0100_0193)
    })
}

/// The layout hash of packed `fields` at record `version`: keys
/// (case-insensitive) and ranges, not defaults.
pub fn layout_hash(version: u8, fields: &[Field]) -> u32 {
    let mut bytes = vec![version];
    for f in fields {
        bytes.extend(f.key.to_ascii_lowercase().bytes());
        bytes.push(0);
        bytes.extend(f.min.to_le_bytes());
        bytes.extend(f.max.to_le_bytes());
    }
    fnv1a(&bytes)
}

/// A save slot of one save profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    /// The host's opaque profile identity.
    pub profile: [u8; 32],
    /// The slot number.
    pub slot: u32,
}

impl Key {
    /// The event's slot and profile, only when the host flagged both valid
    /// and the slot is below `slots`.
    pub fn of(event: &SaveEvent, slots: u32) -> Option<Key> {
        let slot = event.valid_slot().filter(|s| *s < slots)?;
        Some(Key {
            profile: *event.valid_profile()?,
            slot,
        })
    }
}

/// What a record holds besides its header byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    /// The per-save Enabled setting.
    pub enabled: bool,
    /// The adapter's payload (exactly [`Adapter::payload_bytes`] bytes).
    pub payload: Vec<u8>,
}

/// What a unit keeps per save.
pub trait Adapter: Sync {
    /// Record version (`0..=63`), stored in the record's flag byte.
    fn version(&self) -> u8;
    /// The fixed payload size.
    fn payload_bytes(&self) -> usize;
    /// A hash of the payload layout; the region is not read under another.
    fn layout_hash(&self) -> u32;
    /// The current state.
    fn capture(&self) -> Body;
    /// Load a slot's state; `None`: defaults with Enabled off.
    fn restore(&self, body: Option<&Body>);
    /// A new game started (the default keeps everything).
    fn new_game(&self) {}
}

/// Bytes of a record holding `payload` bytes.
pub const fn record_bytes(payload: usize) -> usize {
    1 + payload + 4
}

/// Largest range of an import batch.
const IMPORT_CHUNK: usize = 16 << 10;

/// `ranges` (each at most [`IMPORT_CHUNK`] bytes, in order) grouped into
/// host batches: at most 64 KiB and [`crate::abi::STATE_MAX_RANGES`] each.
fn batches<'a, 'b>(ranges: &'a [(u64, &'b [u8])]) -> Vec<&'a [(u64, &'b [u8])]> {
    let mut out = Vec::new();
    let (mut start, mut size) = (0, 0);
    for (i, (_, bytes)) in ranges.iter().enumerate() {
        if i > start
            && (size + bytes.len() > crate::abi::STATE_MAX_BATCH_BYTES
                || i - start == crate::abi::STATE_MAX_RANGES)
        {
            out.push(&ranges[start..i]);
            (start, size) = (i, 0);
        }
        size += bytes.len();
    }
    if start < ranges.len() {
        out.push(&ranges[start..]);
    }
    out
}

/// Why a stored record is not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// Wrong length.
    Length,
    /// The CRC does not match (corrupt, or another slot's bytes).
    Checksum,
    /// Another record version.
    Version(u8),
}

fn record_crc(key: &Key, flags: u8, payload: &[u8]) -> u32 {
    crc32(&[
        &key.profile,
        &(key.slot as u16).to_le_bytes(),
        &[flags],
        payload,
    ])
}

/// The bytes of `body` for `key` at record `version`.
pub fn encode_record(key: &Key, version: u8, body: &Body) -> Vec<u8> {
    let flags = FLAG_VALID
        | if body.enabled { FLAG_ENABLED } else { 0 }
        | (version.min(MAX_VERSION) << VERSION_SHIFT);
    let mut out = Vec::with_capacity(record_bytes(body.payload.len()));
    out.push(flags);
    out.extend_from_slice(&body.payload);
    out.extend(record_crc(key, flags, &body.payload).to_le_bytes());
    out
}

/// A stored record of `key`: `Ok(None)` for a slot never saved.
pub fn decode_record(key: &Key, version: u8, bytes: &[u8]) -> Result<Option<Body>, RecordError> {
    if bytes.len() < record_bytes(0) {
        return Err(RecordError::Length);
    }
    let flags = bytes[0];
    if flags & FLAG_VALID == 0 {
        return Ok(None);
    }
    let (payload, crc) = bytes[1..].split_at(bytes.len() - 5);
    if record_crc(key, flags, payload) != u32::from_le_bytes([crc[0], crc[1], crc[2], crc[3]]) {
        return Err(RecordError::Checksum);
    }
    let found = flags >> VERSION_SHIFT;
    if found != version {
        return Err(RecordError::Version(found));
    }
    Ok(Some(Body {
        enabled: flags & FLAG_ENABLED != 0,
        payload: payload.to_vec(),
    }))
}

/// The region header and profile directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Slots per profile.
    pub slots: u32,
    /// Bytes per record.
    pub record: usize,
    /// The adapter's layout hash.
    pub hash: u32,
    /// Profile ids in use, in directory order.
    pub profiles: Vec<[u8; 32]>,
    /// Header flags ([`HEADER_IMPORTED`]).
    pub flags: u32,
}

impl Header {
    /// A header of a region not written yet.
    pub fn new(slots: u32, record: usize, hash: u32) -> Header {
        Header {
            slots,
            record,
            hash,
            profiles: Vec::new(),
            flags: 0,
        }
    }

    /// The 32 header bytes.
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut b = [0u8; HEADER_BYTES];
        b[..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&CONTAINER.to_le_bytes());
        b[12..16].copy_from_slice(&self.hash.to_le_bytes());
        b[16..18].copy_from_slice(&(self.slots as u16).to_le_bytes());
        b[18..20].copy_from_slice(&(self.record as u16).to_le_bytes());
        b[20..22].copy_from_slice(&(PROFILES as u16).to_le_bytes());
        b[22..24].copy_from_slice(&(self.profiles.len() as u16).to_le_bytes());
        b[24..28].copy_from_slice(&self.flags.to_le_bytes());
        let crc = crc32(&[&b[..28]]);
        b[28..].copy_from_slice(&crc.to_le_bytes());
        b
    }

    /// Header and directory from the first [`RECORDS_AT`] region bytes.
    pub fn parse(bytes: &[u8]) -> Result<Header, &'static str> {
        if bytes.len() < RECORDS_AT {
            return Err("region shorter than its header");
        }
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        if bytes[..8] != MAGIC {
            return Err("not a per-save region");
        }
        if u32_at(28) != crc32(&[&bytes[..28]]) {
            return Err("header checksum mismatch");
        }
        if u32_at(8) != CONTAINER {
            return Err("unknown container version");
        }
        let count = usize::from(u16_at(22));
        if usize::from(u16_at(20)) != PROFILES || count > PROFILES {
            return Err("bad profile directory");
        }
        let profiles = (0..count)
            .map(|i| {
                let at = HEADER_BYTES + i * PROFILE_BYTES;
                bytes[at..at + PROFILE_BYTES].try_into().unwrap_or([0; 32])
            })
            .collect();
        Ok(Header {
            slots: u32::from(u16_at(16)),
            record: usize::from(u16_at(18)),
            hash: u32_at(12),
            profiles,
            flags: u32_at(24),
        })
    }

    /// Bytes of one profile's record block.
    pub fn block(&self) -> usize {
        self.slots as usize * self.record
    }

    /// Offset of `slot` of directory entry `index`.
    pub fn offset(&self, index: usize, slot: u32) -> u64 {
        (RECORDS_AT + index * self.block() + slot as usize * self.record) as u64
    }

    /// The region length for the profiles in use.
    pub fn length(&self) -> u64 {
        (RECORDS_AT + self.profiles.len() * self.block()) as u64
    }

    /// The directory index of `profile`.
    pub fn index(&self, profile: &[u8; 32]) -> Option<usize> {
        self.profiles.iter().position(|p| p == profile)
    }
}

/// What [`PerSave::open`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// A region never written: records start on the first save.
    Fresh,
    /// A stored region with this many profiles.
    Stored(usize),
    /// No storage service: settings last for the session only.
    NoService,
    /// A region this layout cannot read: kept untouched, nothing persisted.
    Foreign(&'static str),
    /// The host refused to describe the region (status): nothing persisted.
    Failed(i32),
}

/// The store's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Not opened, or no storage service.
    Memory,
    /// A region never written.
    Fresh,
    /// A stored region, readable and writable.
    Ready,
    /// A region another layout wrote: never read or written.
    Foreign,
    /// A write or the region description failed: nothing more is read or
    /// written this session.
    Failed,
}

/// Where the store's log lines go (`level` as [`crate::abi::level`]).
pub type LogSink = fn(level: u32, line: &str);

struct Inner {
    region: Option<Region>,
    mode: Mode,
    readable: bool,
    header: Option<Header>,
    revision: u64,
    pending: BTreeMap<u64, (Key, Vec<u8>)>,
    queue: VecDeque<(Key, Vec<u8>)>,
    deferred: bool,
    log: Option<LogSink>,
    active: Option<Key>,
}

/// One unit's per-save store over its [`Adapter`].
pub struct PerSave {
    unit: &'static str,
    adapter: &'static dyn Adapter,
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl PerSave {
    /// A store for `unit` (its region is `<unit>.per_save`); usable in a `static`.
    pub const fn new(unit: &'static str, adapter: &'static dyn Adapter) -> PerSave {
        PerSave {
            unit,
            adapter,
            inner: Mutex::new(Inner {
                region: None,
                mode: Mode::Memory,
                readable: false,
                header: None,
                revision: 0,
                pending: BTreeMap::new(),
                queue: VecDeque::new(),
                deferred: false,
                log: None,
                active: None,
            }),
            wake: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The unit ID.
    pub const fn unit(&self) -> &'static str {
        self.unit
    }

    /// The adapter.
    pub fn adapter(&self) -> &'static dyn Adapter {
        self.adapter
    }

    /// The region ID to register in `init`.
    pub fn region_id(&self) -> Option<String> {
        region_id(self.unit)
    }

    /// Send log lines to `sink`.
    pub fn set_log(&self, sink: LogSink) {
        self.lock().log = Some(sink);
    }

    /// With `true`, `SAVED` only queues the write for a writer thread
    /// ([`run_writer`](Self::run_writer)); a `LOADED` always finishes
    /// queued writes first. Default: written inside the `SAVED` callback.
    pub fn set_deferred(&self, deferred: bool) {
        self.lock().deferred = deferred;
    }

    /// The state.
    pub fn mode(&self) -> Mode {
        self.lock().mode
    }

    /// The header in use (after a successful open).
    pub fn header(&self) -> Option<Header> {
        self.lock().header.clone()
    }

    /// Captures waiting for their `SAVED`/`SAVE_FAILED`.
    pub fn pending(&self) -> usize {
        self.lock().pending.len()
    }

    /// Writes waiting for [`drain`](Self::drain).
    pub fn queued(&self) -> usize {
        self.lock().queue.len()
    }

    /// The slot last loaded (`None` before a load, after a new game or an
    /// invalid load).
    pub fn active(&self) -> Option<Key> {
        self.lock().active
    }

    fn say(&self, inner: &Inner, level: u32, line: &str) {
        if let Some(sink) = inner.log {
            sink(level, &format!("per-save {}: {line}", self.unit));
        }
    }

    /// Start over `region` (`None`: no storage service) with `slots` save
    /// slots per profile (the save events' slot count). Pending captures,
    /// queued writes and the active slot are forgotten; the adapter's memory
    /// is not touched (at process start it holds its defaults, Enabled off).
    pub fn open(&self, region: Option<Region>, slots: u32) -> Opened {
        let mut inner = self.lock();
        inner.pending.clear();
        inner.queue.clear();
        inner.active = None;
        inner.readable = false;
        inner.header = None;
        inner.revision = 0;
        let payload = self.adapter.payload_bytes();
        let (opened, mode) = match region {
            None => (Opened::NoService, Mode::Memory),
            Some(_) if slots == 0 || slots > MAX_SLOTS => {
                (Opened::Foreign("unsupported slot count"), Mode::Foreign)
            }
            Some(_)
                if payload > MAX_PAYLOAD
                    || self.adapter.version() > MAX_VERSION
                    || (RECORDS_AT + PROFILES * slots as usize * record_bytes(payload)) as u64
                        > crate::abi::STATE_MAX_REGION_BYTES =>
            {
                (Opened::Foreign("unsupported record"), Mode::Foreign)
            }
            Some(region) => {
                let expected =
                    Header::new(slots, record_bytes(payload), self.adapter.layout_hash());
                let result = Self::describe(&region, &expected);
                inner.region = Some(region);
                match result {
                    Err(rc) => (Opened::Failed(rc), Mode::Failed),
                    Ok(None) => {
                        inner.header = Some(expected);
                        (Opened::Fresh, Mode::Fresh)
                    }
                    Ok(Some(Err(why))) => (Opened::Foreign(why), Mode::Foreign),
                    Ok(Some(Ok((header, revision)))) => {
                        let n = header.profiles.len();
                        inner.header = Some(header);
                        inner.revision = revision;
                        inner.readable = true;
                        (Opened::Stored(n), Mode::Ready)
                    }
                }
            }
        };
        inner.mode = mode;
        let line = match opened {
            Opened::Fresh => "new region; records start with the first save".to_owned(),
            Opened::Stored(n) => format!("region with {n} save profile(s)"),
            Opened::NoService => "no host storage: settings last for this session".to_owned(),
            Opened::Foreign(why) => format!("{why}; region left untouched, nothing persisted"),
            Opened::Failed(rc) => format!("region unreadable ({rc}); nothing persisted"),
        };
        let level = match opened {
            Opened::Fresh | Opened::Stored(_) => crate::abi::level::INFO,
            Opened::NoService => crate::abi::level::WARN,
            _ => crate::abi::level::ERROR,
        };
        self.say(&inner, level, &line);
        opened
    }

    /// `Ok(None)`: never written; `Ok(Some(Err))`: unreadable layout.
    #[allow(clippy::type_complexity)]
    fn describe(
        region: &Region,
        expected: &Header,
    ) -> Result<Option<Result<(Header, u64), &'static str>>, i32> {
        let Some(info) = region.info()? else {
            return Ok(None);
        };
        if info.schema != REGION_SCHEMA {
            return Ok(Some(Err("region schema unknown")));
        }
        if info.length < RECORDS_AT as u64 {
            return Ok(Some(Err("region shorter than its header")));
        }
        let mut bytes = vec![0u8; RECORDS_AT];
        region.read(0, &mut bytes)?;
        Ok(Some(Header::parse(&bytes).and_then(|header| {
            if header.hash != expected.hash || header.record != expected.record {
                Err("settings layout changed")
            } else if header.slots != expected.slots {
                Err("slot count changed")
            } else if info.length < header.length() {
                Err("region truncated")
            } else {
                Ok((header, info.revision))
            }
        })))
    }

    /// Seed a region never written with `records` (a one-time import of a
    /// legacy file). Profiles past [`PROFILES`], slots past the slot count
    /// and payloads of another size are skipped. The header and directory
    /// go first, then the record blocks, in revision-checked batches of at
    /// most 64 KiB and [`crate::abi::STATE_MAX_RANGES`] ranges; a batch that
    /// fails stops the import (the slots it did not reach read as "no
    /// record") and nothing more is persisted this session. Returns the
    /// records written.
    pub fn import(&self, records: &[(Key, Body)]) -> Result<usize, i32> {
        let mut inner = self.lock();
        if inner.mode != Mode::Fresh {
            return Err(status::STATE_CONFLICT);
        }
        let (Some(mut header), Some(region)) = (inner.header.clone(), inner.region) else {
            return Err(status::STATE_CONFLICT);
        };
        let mut blocks: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        let mut written = 0;
        for (key, body) in records {
            if key.slot >= header.slots || body.payload.len() != self.adapter.payload_bytes() {
                continue;
            }
            let index = match header.index(&key.profile) {
                Some(i) => i,
                None if header.profiles.len() < PROFILES => {
                    header.profiles.push(key.profile);
                    header.profiles.len() - 1
                }
                None => continue,
            };
            let record = encode_record(key, self.adapter.version(), body);
            let block = blocks
                .entry(index)
                .or_insert_with(|| vec![0; header.block()]);
            let at = key.slot as usize * header.record;
            block[at..at + header.record].copy_from_slice(&record);
            written += 1;
        }
        if blocks.is_empty() {
            return Ok(0);
        }
        header.flags |= HEADER_IMPORTED;
        let mut directory = Vec::with_capacity(header.profiles.len() * PROFILE_BYTES);
        for p in &header.profiles {
            directory.extend_from_slice(p);
        }
        let encoded = header.encode();
        let mut ranges: Vec<(u64, &[u8])> =
            vec![(0, &encoded[..]), (HEADER_BYTES as u64, &directory[..])];
        for (index, block) in &blocks {
            let start = header.offset(*index, 0);
            for (i, piece) in block.chunks(IMPORT_CHUNK).enumerate() {
                ranges.push((start + (i * IMPORT_CHUNK) as u64, piece));
            }
        }
        let length = header.length();
        for batch in batches(&ranges) {
            match region.write_ranges(inner.revision, length, REGION_SCHEMA, batch) {
                Ok(info) => inner.revision = info.revision,
                Err(rc) => {
                    inner.mode = Mode::Failed;
                    inner.readable = false;
                    self.say(
                        &inner,
                        crate::abi::level::ERROR,
                        &format!("import failed ({rc}); nothing more is persisted"),
                    );
                    return Err(rc);
                }
            }
        }
        if let Err(rc) = region.flush() {
            self.say(
                &inner,
                crate::abi::level::WARN,
                &format!("flush after import failed ({rc})"),
            );
        }
        let profiles = header.profiles.len();
        inner.header = Some(header);
        inner.mode = Mode::Ready;
        inner.readable = true;
        self.say(
            &inner,
            crate::abi::level::INFO,
            &format!("imported {written} record(s) over {profiles} save profile(s)"),
        );
        Ok(written)
    }

    /// The stored record of `key` (`None`: none, or unreadable).
    pub fn stored(&self, key: &Key) -> Option<Body> {
        let inner = self.lock();
        self.read_locked(&inner, key)
    }

    fn read_locked(&self, inner: &Inner, key: &Key) -> Option<Body> {
        let (true, Some(header), Some(region)) =
            (inner.readable, inner.header.as_ref(), inner.region)
        else {
            return None;
        };
        let index = header.index(&key.profile)?;
        if key.slot >= header.slots {
            return None;
        }
        let mut bytes = vec![0u8; header.record];
        if let Err(rc) = region.read(header.offset(index, key.slot), &mut bytes) {
            self.say(
                inner,
                crate::abi::level::ERROR,
                &format!("slot {} unreadable ({rc}); defaults, off", key.slot),
            );
            return None;
        }
        // The record size was checked against the adapter when the region opened.
        match decode_record(key, self.adapter.version(), &bytes) {
            Ok(body) => body,
            Err(e) => {
                self.say(
                    inner,
                    crate::abi::level::ERROR,
                    &format!("slot {} record rejected ({e:?}); defaults, off", key.slot),
                );
                None
            }
        }
    }

    /// Write every queued record (revision-checked batch + flush each).
    /// Returns the records written.
    pub fn drain(&self) -> usize {
        let mut inner = self.lock();
        self.drain_locked(&mut inner)
    }

    fn drain_locked(&self, inner: &mut Inner) -> usize {
        let mut written = 0;
        while let Some((key, record)) = inner.queue.pop_front() {
            if !matches!(inner.mode, Mode::Fresh | Mode::Ready) {
                self.say(
                    inner,
                    crate::abi::level::WARN,
                    &format!(
                        "slot {} not persisted (store is {:?})",
                        key.slot, inner.mode
                    ),
                );
                continue;
            }
            if self.write_locked(inner, &key, &record) {
                written += 1;
            }
        }
        written
    }

    fn write_locked(&self, inner: &mut Inner, key: &Key, record: &[u8]) -> bool {
        let (Some(region), Some(mut header)) = (inner.region, inner.header.clone()) else {
            return false;
        };
        let prior = header.clone();
        let (index, grows) = match header.index(&key.profile) {
            Some(i) => (i, false),
            None if header.profiles.len() < PROFILES => {
                header.profiles.push(key.profile);
                (header.profiles.len() - 1, true)
            }
            None => {
                self.say(
                    inner,
                    crate::abi::level::ERROR,
                    &format!(
                        "save profile directory full; slot {} not persisted",
                        key.slot
                    ),
                );
                return false;
            }
        };
        let head = header.encode();
        let entry = key.profile;
        let mut ranges: Vec<(u64, &[u8])> = Vec::with_capacity(3);
        if grows || inner.mode == Mode::Fresh {
            ranges.push((0, &head[..]));
            ranges.push(((HEADER_BYTES + index * PROFILE_BYTES) as u64, &entry[..]));
        }
        ranges.push((header.offset(index, key.slot), record));
        let length = header.length();
        let mut result = region.write_ranges(inner.revision, length, REGION_SCHEMA, &ranges);
        if result == Err(status::STATE_CONFLICT)
            && let Some(revision) = self.recheck(&region, &prior, inner.mode)
        {
            inner.revision = revision;
            result = region.write_ranges(inner.revision, length, REGION_SCHEMA, &ranges);
        }
        match result {
            Ok(info) => {
                inner.revision = info.revision;
                inner.header = Some(header);
                inner.mode = Mode::Ready;
                inner.readable = true;
                if let Err(rc) = region.flush() {
                    self.say(
                        inner,
                        crate::abi::level::WARN,
                        &format!("flush failed ({rc})"),
                    );
                }
                self.say(
                    inner,
                    crate::abi::level::INFO,
                    &format!("slot {} saved", key.slot),
                );
                true
            }
            Err(rc) => {
                inner.mode = Mode::Failed;
                inner.readable = false;
                self.say(
                    inner,
                    crate::abi::level::ERROR,
                    &format!(
                        "slot {} write failed ({rc}); nothing more is persisted this session",
                        key.slot
                    ),
                );
                false
            }
        }
    }

    /// After a revision conflict: the new revision when the stored header
    /// still matches ours (then a retry is safe).
    fn recheck(&self, region: &Region, prior: &Header, mode: Mode) -> Option<u64> {
        let info = region.info().ok()??;
        if mode == Mode::Fresh {
            return None;
        }
        let mut bytes = vec![0u8; RECORDS_AT];
        region.read(0, &mut bytes).ok()?;
        (Header::parse(&bytes).ok()? == *prior).then_some(info.revision)
    }

    /// The writer thread body for [`set_deferred`](Self::set_deferred)`(true)`:
    /// waits for queued records and writes them, forever.
    pub fn run_writer(&self) -> ! {
        loop {
            self.write_queued();
        }
    }

    /// One writer pass: wait until a record is queued, then write every
    /// queued record. Returns the records written.
    pub fn write_queued(&self) -> usize {
        let mut inner = self.lock();
        while inner.queue.is_empty() {
            inner = self.wake.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        self.drain_locked(&mut inner)
    }

    /// Handle one save event (the unit's save-event handler calls this).
    /// Returns 0, or a host status the host logs (advisory).
    pub fn on_event(&self, event: &SaveEvent) -> i32 {
        let Some(kind) = event.event_kind() else {
            self.say(
                &self.lock(),
                crate::abi::level::WARN,
                &format!("unknown save event kind {}", event.kind),
            );
            return status::INVALID_ARGUMENT;
        };
        match kind {
            EventKind::SaveBegin => self.save_begin(event),
            EventKind::Saved | EventKind::SaveFailed => {
                self.save_end(event, kind == EventKind::Saved)
            }
            EventKind::Loaded => self.loaded(event),
            EventKind::NewGame => {
                self.adapter.new_game();
                let mut inner = self.lock();
                inner.active = None;
                self.say(
                    &inner,
                    crate::abi::level::INFO,
                    "new game: settings kept from the title screen",
                );
                status::OK
            }
        }
    }

    fn slots(&self, inner: &Inner) -> u32 {
        inner.header.as_ref().map_or(MAX_SLOTS, |h| h.slots)
    }

    fn save_begin(&self, event: &SaveEvent) -> i32 {
        let slots = self.slots(&self.lock());
        let Some(key) = Key::of(event, slots) else {
            self.say(
                &self.lock(),
                crate::abi::level::WARN,
                "save without a valid slot/profile: not captured",
            );
            return status::INVALID_ARGUMENT;
        };
        if event.operation == 0 {
            self.say(
                &self.lock(),
                crate::abi::level::WARN,
                "save without an operation id: not captured",
            );
            return status::INVALID_ARGUMENT;
        }
        let record = encode_record(&key, self.adapter.version(), &self.adapter.capture());
        let mut inner = self.lock();
        if inner.pending.contains_key(&event.operation) {
            self.say(
                &inner,
                crate::abi::level::WARN,
                &format!(
                    "operation {} began twice; first capture kept",
                    event.operation
                ),
            );
            return status::INVALID_ARGUMENT;
        }
        if inner.pending.len() >= MAX_PENDING {
            self.say(
                &inner,
                crate::abi::level::ERROR,
                "too many unfinished saves; capture dropped",
            );
            return status::STATE_LIMIT;
        }
        inner.pending.insert(event.operation, (key, record));
        status::OK
    }

    fn save_end(&self, event: &SaveEvent, saved: bool) -> i32 {
        let mut inner = self.lock();
        let Some((key, record)) = inner.pending.remove(&event.operation) else {
            self.say(
                &inner,
                crate::abi::level::WARN,
                &format!("save operation {} unknown; ignored", event.operation),
            );
            return status::NOT_FOUND;
        };
        let slots = self.slots(&inner);
        if Key::of(event, slots) != Some(key) {
            self.say(
                &inner,
                crate::abi::level::WARN,
                &format!("save operation {} changed slot; discarded", event.operation),
            );
            return status::INVALID_ARGUMENT;
        }
        if !saved {
            self.say(
                &inner,
                crate::abi::level::INFO,
                &format!("slot {} save failed; capture discarded", key.slot),
            );
            return status::OK;
        }
        inner.queue.retain(|(k, _)| *k != key);
        inner.queue.push_back((key, record));
        if inner.deferred {
            self.wake.notify_all();
        } else {
            self.drain_locked(&mut inner);
        }
        status::OK
    }

    fn loaded(&self, event: &SaveEvent) -> i32 {
        let mut inner = self.lock();
        self.drain_locked(&mut inner);
        let key = Key::of(event, self.slots(&inner));
        let body = key.and_then(|k| self.read_locked(&inner, &k));
        inner.active = key;
        let line = match (key, &body) {
            (None, _) => "load without a valid slot/profile: defaults, off".to_owned(),
            (Some(k), Some(b)) => format!(
                "slot {} restored ({})",
                k.slot,
                if b.enabled { "on" } else { "off" }
            ),
            (Some(k), None) => format!("slot {} has no record: defaults, off", k.slot),
        };
        let level = if key.is_some() {
            crate::abi::level::INFO
        } else {
            crate::abi::level::WARN
        };
        self.say(&inner, level, &line);
        drop(inner);
        self.adapter.restore(body.as_ref());
        if key.is_some() {
            status::OK
        } else {
            status::INVALID_ARGUMENT
        }
    }
}

/// The per-save [`Adapter`] over a set of [`Param`]s: the Enabled param
/// in the record's flag byte, the others as a packed record
/// ([`crate::packed`], schema 1, without its version byte; the version is
/// the record version). Restores go through [`Param::set`], so change
/// callbacks run; nothing touches the disk outside save events.
pub struct Params {
    version: u8,
    enabled: &'static Param,
    params: &'static [&'static Param],
    fields: OnceLock<&'static [Field]>,
}

impl Params {
    /// The adapter of `enabled` and `params` at record `version` (`0..=63`).
    pub const fn new(
        version: u8,
        enabled: &'static Param,
        params: &'static [&'static Param],
    ) -> Params {
        Params {
            version,
            enabled,
            params,
            fields: OnceLock::new(),
        }
    }

    /// The Enabled param.
    pub fn enabled(&self) -> &'static Param {
        self.enabled
    }

    /// The other params, in record order.
    pub fn params(&self) -> &'static [&'static Param] {
        self.params
    }

    /// The packed fields (built once).
    pub fn fields(&self) -> &'static [Field] {
        self.fields.get_or_init(|| {
            Box::leak(
                self.params
                    .iter()
                    .map(|p| p.field())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            )
        })
    }

    /// The packed layout (version byte = the record version, schema 1).
    pub fn layout(&self) -> Layout {
        Layout {
            version: self.version,
            schema: 1,
            fields: self.fields(),
        }
    }

    /// Every param to its default and Enabled off.
    pub fn defaults(&self) {
        for p in self.params {
            p.reset();
        }
        self.enabled.set(0);
    }

    fn values(&self, payload: &[u8]) -> Option<Vec<i64>> {
        let mut bytes = Vec::with_capacity(payload.len() + 1);
        bytes.push(self.version);
        bytes.extend_from_slice(payload);
        let (values, text) = packed::decode(&self.layout(), &bytes)?;
        (text.is_empty() && bytes.len() == self.layout().packed_len()).then_some(values)
    }
}

impl Adapter for Params {
    fn version(&self) -> u8 {
        self.version
    }

    fn payload_bytes(&self) -> usize {
        self.layout().packed_len() - 1
    }

    fn layout_hash(&self) -> u32 {
        layout_hash(self.version, self.fields())
    }

    fn capture(&self) -> Body {
        let values: Vec<i64> = self.params.iter().map(|p| i64::from(p.get())).collect();
        let bytes = packed::encode(&self.layout(), &values, &[]).unwrap_or_default();
        Body {
            enabled: self.enabled.is_on(),
            payload: bytes.get(1..).map(<[u8]>::to_vec).unwrap_or_default(),
        }
    }

    fn restore(&self, body: Option<&Body>) {
        let Some((body, values)) = body.and_then(|b| Some((b, self.values(&b.payload)?))) else {
            self.defaults();
            return;
        };
        for (p, v) in self.params.iter().zip(values) {
            p.set(v as u32);
        }
        self.enabled.set(u32::from(body.enabled));
    }
}

#[cfg(test)]
#[path = "per_save_tests.rs"]
mod tests;

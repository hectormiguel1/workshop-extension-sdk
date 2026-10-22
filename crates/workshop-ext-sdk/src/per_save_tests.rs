//! Every per-save transition over the in-memory host: process start, menu
//! edits, save begin/saved/failed, loads with and without records, new
//! games, invalid identities, foreign and failed regions, the profile
//! directory, the one-time import and the deferred writer.
use super::*;
use crate::abi::{self, LOADED, NEW_GAME, SAVE_BEGIN, SAVE_FAILED, SAVED};
use crate::testing::{self, FakeHost};
use std::cell::RefCell;

const ID: &str = "test.unit.per_save";
const P1: [u8; 32] = [1; 32];
const P2: [u8; 32] = [2; 32];

thread_local! {
    static REGION: core::cell::Cell<Option<Region>> = const { core::cell::Cell::new(None) };
    static LINES: RefCell<Vec<(u32, String)>> = const { RefCell::new(Vec::new()) };
}

fn sink(level: u32, line: &str) {
    LINES.with(|l| l.borrow_mut().push((level, line.to_owned())));
}

fn said(text: &str) -> bool {
    LINES.with(|l| l.borrow().iter().any(|(_, line)| line.contains(text)))
}

fn host() -> FakeHost {
    LINES.with(|l| l.borrow_mut().clear());
    REGION.with(|r| r.set(None));
    FakeHost::new(abi::GAME_XIII)
}

/// The test's region (registered once per fake host, like `init`).
fn region() -> Region {
    REGION.with(|r| {
        let region = r
            .get()
            .unwrap_or_else(|| Region::register(&testing::STATE_API, ID).unwrap());
        r.set(Some(region));
        region
    })
}

fn ev(kind: u32, slot: u32, profile: [u8; 32], operation: u64) -> SaveEvent {
    testing::event(kind, Some(slot), Some(profile), operation)
}

/// A store over three params, fresh statics per expansion.
macro_rules! fixture {
    () => {{
        static ENABLED: Param = Param::flag("Enabled", false);
        static LEVEL: Param = Param::new("Level", 3, 1, 9);
        static GIL: Param = Param::new("Gil", 100, 0, 1000);
        static PARAMS: Params = Params::new(1, &ENABLED, &[&LEVEL, &GIL]);
        static STORE: PerSave = PerSave::new("test.unit", &PARAMS);
        STORE.set_log(sink);
        (&STORE, &ENABLED, &LEVEL, &GIL)
    }};
}

/// Save `slot` of `profile` through a full SAVE_BEGIN/SAVED pair.
fn save(store: &PerSave, slot: u32, profile: [u8; 32], operation: u64) -> (i32, i32) {
    (
        store.on_event(&ev(SAVE_BEGIN, slot, profile, operation)),
        store.on_event(&ev(SAVED, slot, profile, operation)),
    )
}

#[test]
fn checksums_and_hashes_are_the_documented_algorithms() {
    assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926);
    assert_eq!(crc32(&[b"1234", b"56789"]), 0xcbf4_3926);
    assert_eq!(fnv1a(b""), 0x811c_9dc5);
    assert_eq!(fnv1a(b"a"), 0xe40c_292c);
    let a = [Field::new("Level", 3, 1, 9)];
    let b = [Field::new("LEVEL", 5, 1, 9)];
    let c = [Field::new("Level", 3, 1, 10)];
    assert_eq!(
        layout_hash(1, &a),
        layout_hash(1, &b),
        "keys case-insensitive, defaults ignored"
    );
    assert_ne!(layout_hash(1, &a), layout_hash(1, &c), "ranges count");
    assert_ne!(layout_hash(1, &a), layout_hash(2, &a), "versions count");
    assert_eq!(region_id("x.y").as_deref(), Some("x.y.per_save"));
    assert_eq!(region_id(&"x".repeat(60)), None, "longer than 64 bytes");
    assert_eq!(region_id("bad id"), None);
    assert_eq!(record_bytes(3), 8);
}

#[test]
fn records_round_trip_and_reject_other_slots_versions_and_damage() {
    let key = Key {
        profile: P1,
        slot: 4,
    };
    let body = Body {
        enabled: true,
        payload: vec![9, 8, 7],
    };
    let bytes = encode_record(&key, 5, &body);
    assert_eq!(bytes.len(), record_bytes(3));
    assert_eq!(bytes[0], FLAG_VALID | FLAG_ENABLED | (5 << VERSION_SHIFT));
    assert_eq!(decode_record(&key, 5, &bytes), Ok(Some(body.clone())));
    let off = Body {
        enabled: false,
        payload: vec![1, 2, 3],
    };
    assert_eq!(
        decode_record(&key, 5, &encode_record(&key, 5, &off)),
        Ok(Some(off))
    );
    assert_eq!(
        decode_record(&key, 5, &[0; 8]),
        Ok(None),
        "a slot never saved"
    );
    assert_eq!(decode_record(&key, 5, &[1, 2]), Err(RecordError::Length));
    let other_slot = Key {
        profile: P1,
        slot: 5,
    };
    assert_eq!(
        decode_record(&other_slot, 5, &bytes),
        Err(RecordError::Checksum)
    );
    let other_profile = Key {
        profile: P2,
        slot: 4,
    };
    assert_eq!(
        decode_record(&other_profile, 5, &bytes),
        Err(RecordError::Checksum)
    );
    assert_eq!(decode_record(&key, 6, &bytes), Err(RecordError::Version(5)));
    let mut damaged = bytes.clone();
    damaged[2] ^= 1;
    assert_eq!(decode_record(&key, 5, &damaged), Err(RecordError::Checksum));
    // The version saturates at 63.
    assert_eq!(
        encode_record(&key, 200, &body)[0] >> VERSION_SHIFT,
        MAX_VERSION
    );
}

#[test]
fn headers_round_trip_and_reject_damage() {
    let mut header = Header::new(99, 24, 0xdead_beef);
    header.profiles = vec![P1, P2];
    header.flags = HEADER_IMPORTED;
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(&P1);
    bytes.extend_from_slice(&P2);
    bytes.resize(RECORDS_AT, 0);
    assert_eq!(Header::parse(&bytes), Ok(header.clone()));
    assert_eq!(header.block(), 99 * 24);
    assert_eq!(header.offset(1, 2), (RECORDS_AT + 99 * 24 + 48) as u64);
    assert_eq!(header.length(), (RECORDS_AT + 2 * 99 * 24) as u64);
    assert_eq!((header.index(&P2), header.index(&[3; 32])), (Some(1), None));
    assert_eq!(
        Header::parse(&bytes[..100]),
        Err("region shorter than its header")
    );
    let mut magic = bytes.clone();
    magic[0] = b'X';
    assert_eq!(Header::parse(&magic), Err("not a per-save region"));
    let mut crc = bytes.clone();
    crc[16] ^= 1;
    assert_eq!(Header::parse(&crc), Err("header checksum mismatch"));
    let reseal = |b: &mut Vec<u8>| {
        let c = crc32(&[&b[..28]]);
        b[28..32].copy_from_slice(&c.to_le_bytes());
    };
    let mut version = bytes.clone();
    version[8] = 2;
    reseal(&mut version);
    assert_eq!(Header::parse(&version), Err("unknown container version"));
    let mut capacity = bytes.clone();
    capacity[20] = 8;
    reseal(&mut capacity);
    assert_eq!(Header::parse(&capacity), Err("bad profile directory"));
    let mut count = bytes;
    count[22] = 17;
    reseal(&mut count);
    assert_eq!(Header::parse(&count), Err("bad profile directory"));
}

#[test]
fn keys_need_a_valid_slot_and_profile_below_the_slot_count() {
    let good = ev(SAVED, 3, P1, 1);
    assert_eq!(
        Key::of(&good, 99),
        Some(Key {
            profile: P1,
            slot: 3
        })
    );
    assert_eq!(Key::of(&good, 3), None, "slot past the count");
    assert_eq!(Key::of(&testing::event(SAVED, Some(3), None, 1), 99), None);
    assert_eq!(Key::of(&testing::event(SAVED, None, Some(P1), 1), 99), None);
}

#[test]
fn process_start_is_defaults_off_and_menu_edits_stay_in_memory() {
    let _h = host();
    let (store, enabled, level, gil) = fixture!();
    assert_eq!(store.open(Some(region()), 99), Opened::Fresh);
    assert!(said("new region"));
    assert_eq!((enabled.is_on(), level.get(), gil.get()), (false, 3, 100));
    assert_eq!(store.mode(), Mode::Fresh);
    assert_eq!(store.unit(), "test.unit");
    assert_eq!(
        store.adapter().payload_bytes(),
        store.header().unwrap().record - 5
    );
    enabled.set(1);
    level.set(7);
    assert_eq!(FakeHost::regions()[0].schema, 0, "menu edits never write");
    assert_eq!(store.active(), None);
}

#[test]
fn a_save_writes_the_slot_and_a_load_restores_it() {
    let _h = host();
    let (store, enabled, level, gil) = fixture!();
    store.open(Some(region()), 99);
    enabled.set(1);
    level.set(7);
    gil.set(555);
    assert_eq!(store.on_event(&ev(SAVE_BEGIN, 2, P1, 10)), status::OK);
    assert_eq!(store.pending(), 1);
    level.set(9); // after SAVE_BEGIN: not in this save
    assert_eq!(store.on_event(&ev(SAVED, 2, P1, 10)), status::OK);
    assert_eq!((store.pending(), store.queued()), (0, 0));
    assert_eq!(store.mode(), Mode::Ready);
    assert!(FakeHost::flushes() >= 1);
    assert!(said("slot 2 saved"));
    let stored = store
        .stored(&Key {
            profile: P1,
            slot: 2,
        })
        .unwrap();
    assert!(stored.enabled);
    // Another slot without a record: defaults, off.
    assert_eq!(store.on_event(&ev(LOADED, 5, P1, 11)), status::OK);
    assert_eq!((enabled.is_on(), level.get(), gil.get()), (false, 3, 100));
    assert!(said("slot 5 has no record"));
    assert_eq!(
        store.active(),
        Some(Key {
            profile: P1,
            slot: 5
        })
    );
    // The saved slot comes back as captured at SAVE_BEGIN.
    store.on_event(&ev(LOADED, 2, P1, 12));
    assert_eq!((enabled.is_on(), level.get(), gil.get()), (true, 7, 555));
    assert!(said("slot 2 restored (on)"));
    // A later save of the same slot replaces it in place.
    let length = FakeHost::regions()[0].bytes.len();
    enabled.set(0);
    save(store, 2, P1, 13);
    assert_eq!(FakeHost::regions()[0].bytes.len(), length);
    store.on_event(&ev(LOADED, 2, P1, 14));
    assert!(!enabled.is_on());
    assert!(said("slot 2 restored (off)"));
}

#[test]
fn the_next_session_reads_the_stored_region() {
    let _h = host();
    let (store, enabled, level, _) = fixture!();
    store.open(Some(region()), 99);
    enabled.set(1);
    level.set(8);
    save(store, 0, P1, 1);
    let stored = FakeHost::take_region(ID).unwrap();
    let _h = host();
    FakeHost::seed_region(ID, stored.schema, &stored.bytes);
    let (next, enabled, level, _) = fixture!();
    assert_eq!(next.open(Some(region()), 99), Opened::Stored(1));
    assert!(said("region with 1 save profile(s)"));
    assert_eq!(
        (enabled.is_on(), level.get()),
        (false, 3),
        "process start is off"
    );
    next.on_event(&ev(LOADED, 0, P1, 2));
    assert_eq!((enabled.is_on(), level.get()), (true, 8));
}

#[test]
fn a_failed_save_discards_its_capture_and_unknown_operations_are_ignored() {
    let _h = host();
    let (store, enabled, _, _) = fixture!();
    store.open(Some(region()), 99);
    enabled.set(1);
    store.on_event(&ev(SAVE_BEGIN, 1, P1, 20));
    assert_eq!(store.on_event(&ev(SAVE_FAILED, 1, P1, 20)), status::OK);
    assert_eq!(store.pending(), 0);
    assert!(said("slot 1 save failed; capture discarded"));
    assert_eq!(FakeHost::regions()[0].schema, 0, "nothing written");
    assert_eq!(store.on_event(&ev(SAVED, 1, P1, 99)), status::NOT_FOUND);
    assert!(said("save operation 99 unknown; ignored"));
    assert_eq!(
        store.on_event(&ev(SAVE_FAILED, 1, P1, 98)),
        status::NOT_FOUND
    );
    // The same operation id twice: the first capture is kept.
    store.on_event(&ev(SAVE_BEGIN, 1, P1, 21));
    assert_eq!(
        store.on_event(&ev(SAVE_BEGIN, 1, P1, 21)),
        status::INVALID_ARGUMENT
    );
    assert!(said("operation 21 began twice"));
    // A SAVED for another slot than its SAVE_BEGIN is discarded.
    assert_eq!(
        store.on_event(&ev(SAVED, 2, P1, 21)),
        status::INVALID_ARGUMENT
    );
    assert!(said("changed slot; discarded"));
    assert_eq!(FakeHost::regions()[0].schema, 0);
    // An unknown event kind is ignored.
    let mut odd = ev(SAVED, 1, P1, 22);
    odd.kind = 77;
    assert_eq!(store.on_event(&odd), status::INVALID_ARGUMENT);
    assert!(said("unknown save event kind 77"));
}

#[test]
fn captures_are_bounded_and_need_an_identity_and_an_operation() {
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 99);
    for op in 1..=MAX_PENDING as u64 {
        assert_eq!(store.on_event(&ev(SAVE_BEGIN, 1, P1, op)), status::OK);
    }
    assert_eq!(
        store.on_event(&ev(SAVE_BEGIN, 1, P1, 1000)),
        status::STATE_LIMIT
    );
    assert!(said("too many unfinished saves"));
    assert_eq!(store.pending(), MAX_PENDING);
    assert_eq!(
        store.on_event(&ev(SAVE_BEGIN, 1, P1, 0)),
        status::INVALID_ARGUMENT
    );
    assert!(said("without an operation id"));
    let no_profile = testing::event(SAVE_BEGIN, Some(1), None, 5000);
    assert_eq!(store.on_event(&no_profile), status::INVALID_ARGUMENT);
    assert!(said("save without a valid slot/profile"));
    assert_eq!(
        store.on_event(&ev(SAVE_BEGIN, 99, P1, 5001)),
        status::INVALID_ARGUMENT,
        "slot past the count"
    );
    // Reopening forgets every capture.
    store.open(Some(region()), 99);
    assert_eq!(store.pending(), 0);
}

#[test]
fn an_invalid_load_is_off_and_never_slot_zero() {
    let _h = host();
    let (store, enabled, level, _) = fixture!();
    store.open(Some(region()), 99);
    enabled.set(1);
    level.set(9);
    save(store, 0, P1, 1);
    let no_slot = testing::event(LOADED, None, Some(P1), 2);
    assert_eq!(store.on_event(&no_slot), status::INVALID_ARGUMENT);
    assert_eq!((enabled.is_on(), level.get()), (false, 3));
    assert!(said("load without a valid slot/profile: defaults, off"));
    assert_eq!(store.active(), None);
    enabled.set(1);
    let no_profile = testing::event(LOADED, Some(0), None, 3);
    store.on_event(&no_profile);
    assert!(!enabled.is_on(), "slot 0 of no profile is not slot 0");
    // Another profile's slot 0 is a different record.
    enabled.set(1);
    store.on_event(&ev(LOADED, 0, P2, 4));
    assert!(!enabled.is_on());
}

#[test]
fn a_new_game_keeps_memory_and_quitting_to_title_changes_nothing() {
    let _h = host();
    let (store, enabled, level, _) = fixture!();
    store.open(Some(region()), 99);
    store.on_event(&ev(LOADED, 3, P1, 1));
    enabled.set(1);
    level.set(5);
    // Quit to title: no event at all; memory as it was.
    assert_eq!((enabled.is_on(), level.get()), (true, 5));
    assert_eq!(
        store.on_event(&testing::event(NEW_GAME, None, Some(P1), 2)),
        status::OK
    );
    assert_eq!((enabled.is_on(), level.get()), (true, 5));
    assert_eq!(store.active(), None);
    assert!(said("new game: settings kept"));
    assert_eq!(FakeHost::regions()[0].schema, 0);
}

#[test]
fn profiles_are_kept_apart_up_to_the_directory_capacity() {
    let _h = host();
    let (store, enabled, _, _) = fixture!();
    store.open(Some(region()), 4);
    enabled.set(1);
    save(store, 1, P1, 1);
    enabled.set(0);
    save(store, 1, P2, 2);
    assert_eq!(store.header().unwrap().profiles, vec![P1, P2]);
    store.on_event(&ev(LOADED, 1, P1, 3));
    assert!(enabled.is_on());
    store.on_event(&ev(LOADED, 1, P2, 4));
    assert!(!enabled.is_on());
    for p in 3..=PROFILES as u8 {
        let (_, saved) = save(store, 0, [p; 32], 10 + u64::from(p));
        assert_eq!(saved, status::OK);
    }
    assert_eq!(store.header().unwrap().profiles.len(), PROFILES);
    save(store, 0, [99; 32], 100);
    assert!(said("save profile directory full; slot 0 not persisted"));
    assert_eq!(store.mode(), Mode::Ready, "the store stays usable");
    let length = FakeHost::regions()[0].bytes.len() as u64;
    assert_eq!(length, store.header().unwrap().length());
}

#[test]
fn a_revision_conflict_retries_once_when_the_header_is_unchanged() {
    let _h = host();
    let (store, enabled, _, _) = fixture!();
    store.open(Some(region()), 8);
    save(store, 0, P1, 1);
    // Another writer bumped the revision without touching the header.
    FakeHost::edit_region(ID, |r| r.revision += 5);
    enabled.set(1);
    let (_, saved) = save(store, 1, P1, 2);
    assert_eq!(saved, status::OK);
    assert_eq!(store.mode(), Mode::Ready);
    assert!(
        store
            .stored(&Key {
                profile: P1,
                slot: 1
            })
            .unwrap()
            .enabled
    );
    // A changed header is never overwritten.
    FakeHost::edit_region(ID, |r| {
        r.revision += 1;
        r.bytes[16] ^= 1;
    });
    save(store, 2, P1, 3);
    assert_eq!(store.mode(), Mode::Failed);
    assert!(said("slot 2 write failed"));
    // Nothing more is written this session.
    let before = FakeHost::regions()[0].revision;
    save(store, 3, P1, 4);
    assert_eq!(FakeHost::regions()[0].revision, before);
    assert!(said("slot 3 not persisted (store is Failed)"));
    // A failed store no longer reads either.
    store.on_event(&ev(LOADED, 1, P1, 5));
    assert!(!enabled.is_on());
}

#[test]
fn a_conflict_on_the_first_write_or_a_write_error_stops_persisting() {
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 8);
    FakeHost::edit_region(ID, |r| r.revision = 3);
    save(store, 0, P1, 1);
    assert_eq!(
        store.mode(),
        Mode::Failed,
        "a fresh region written elsewhere"
    );
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 8);
    FakeHost::fail_services(0, status::STATE_IO_ERROR, 0, 0);
    save(store, 0, P1, 1);
    assert_eq!(store.mode(), Mode::Failed);
    assert!(said("write failed (-7)"));
}

#[test]
fn a_flush_failure_is_logged_and_the_write_kept() {
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 8);
    FakeHost::fail_flush(status::STATE_IO_ERROR);
    save(store, 0, P1, 1);
    assert!(said("flush failed (-7)"));
    assert_eq!(store.mode(), Mode::Ready);
    assert!(
        store
            .stored(&Key {
                profile: P1,
                slot: 0
            })
            .is_some()
    );
}

type Damage = Box<dyn Fn(&mut Vec<u8>)>;

#[test]
fn foreign_regions_are_never_read_or_written() {
    let cases: [(&str, Damage); 6] = [
        ("region schema unknown", Box::new(|_| {})),
        (
            "region shorter than its header",
            Box::new(|b| b.truncate(100)),
        ),
        ("not a per-save region", Box::new(|b| b[0] = b'Z')),
        (
            "settings layout changed",
            Box::new(|b| {
                b[12] ^= 1;
                let c = crc32(&[&b[..28]]);
                b[28..32].copy_from_slice(&c.to_le_bytes());
            }),
        ),
        (
            "slot count changed",
            Box::new(|b| {
                b[16] = 7;
                let c = crc32(&[&b[..28]]);
                b[28..32].copy_from_slice(&c.to_le_bytes());
            }),
        ),
        ("region truncated", Box::new(|b| b.truncate(RECORDS_AT + 3))),
    ];
    for (i, (why, damage)) in cases.iter().enumerate() {
        let _h = host();
        let (store, enabled, _, _) = fixture!();
        store.open(Some(region()), 8);
        enabled.set(1);
        save(store, 0, P1, 1);
        let mut stored = FakeHost::take_region(ID).unwrap();
        damage(&mut stored.bytes);
        let schema = if i == 0 { 9 } else { stored.schema };
        let _h = host();
        FakeHost::seed_region(ID, schema, &stored.bytes);
        let (store, enabled, _, _) = fixture!();
        assert_eq!(store.open(Some(region()), 8), Opened::Foreign(why), "{why}");
        assert_eq!(store.mode(), Mode::Foreign);
        assert!(said("region left untouched"));
        save(store, 0, P1, 2);
        store.on_event(&ev(LOADED, 0, P1, 3));
        assert!(!enabled.is_on(), "{why}: never read");
        let after = FakeHost::regions()[0].clone();
        assert_eq!(
            (after.bytes, after.revision),
            (stored.bytes.clone(), 1),
            "{why}: never written"
        );
        assert_eq!(store.import(&[]), Err(status::STATE_CONFLICT));
    }
}

#[test]
fn unsupported_shapes_no_service_and_host_errors() {
    let _h = host();
    let (store, enabled, _, _) = fixture!();
    assert_eq!(store.open(None, 99), Opened::NoService);
    assert_eq!(store.mode(), Mode::Memory);
    assert!(said("no host storage"));
    enabled.set(1);
    save(store, 0, P1, 1);
    assert!(said("slot 0 not persisted (store is Memory)"));
    store.on_event(&ev(LOADED, 0, P1, 2));
    assert!(!enabled.is_on());
    assert_eq!(
        store.open(Some(region()), 0),
        Opened::Foreign("unsupported slot count")
    );
    assert_eq!(
        store.open(Some(region()), MAX_SLOTS + 1),
        Opened::Foreign("unsupported slot count")
    );
    // A short StateInfo from the host: the region cannot be described.
    let _h = host();
    let (store, _, _, _) = fixture!();
    let r = region();
    store.open(Some(r), 8);
    save(store, 0, P1, 4);
    FakeHost::info_size(4);
    assert_eq!(
        store.open(Some(r), 8),
        Opened::Failed(status::DECODE_FAILED)
    );
    assert_eq!(store.mode(), Mode::Failed);
    assert!(said("region unreadable (-4)"));
}

/// A raw adapter with an oversized or invalid record.
struct Raw {
    version: u8,
    bytes: usize,
}
impl Adapter for Raw {
    fn version(&self) -> u8 {
        self.version
    }
    fn payload_bytes(&self) -> usize {
        self.bytes
    }
    fn layout_hash(&self) -> u32 {
        7
    }
    fn capture(&self) -> Body {
        Body {
            enabled: true,
            payload: vec![0x5a; self.bytes],
        }
    }
    fn restore(&self, _: Option<&Body>) {}
}

#[test]
fn records_larger_than_the_contract_are_refused() {
    let _h = host();
    static BIG: Raw = Raw {
        version: 1,
        bytes: MAX_PAYLOAD + 1,
    };
    static OLD: Raw = Raw {
        version: 64,
        bytes: 4,
    };
    static WIDE: Raw = Raw {
        version: 1,
        bytes: 1000,
    };
    for (adapter, slots) in [(&BIG as &'static dyn Adapter, 8), (&OLD, 8), (&WIDE, 999)] {
        let store: &'static PerSave = Box::leak(Box::new(PerSave::new("raw", adapter)));
        assert_eq!(
            store.open(Some(region()), slots),
            Opened::Foreign("unsupported record")
        );
    }
    // The default new_game keeps everything.
    BIG.new_game();
}

#[test]
fn unreadable_or_corrupt_records_load_as_defaults_off() {
    let _h = host();
    let (store, enabled, _, _) = fixture!();
    store.open(Some(region()), 8);
    enabled.set(1);
    save(store, 2, P1, 1);
    let key = Key {
        profile: P1,
        slot: 2,
    };
    let at = store.header().unwrap().offset(0, 2) as usize;
    FakeHost::edit_region(ID, |r| r.bytes[at + 1] ^= 0xff);
    assert_eq!(store.stored(&key), None);
    assert!(said("slot 2 record rejected (Checksum)"));
    store.on_event(&ev(LOADED, 2, P1, 2));
    assert!(!enabled.is_on());
    // A read error (the bytes are gone) is logged too.
    FakeHost::edit_region(ID, |r| r.bytes.truncate(RECORDS_AT));
    assert_eq!(store.stored(&key), None);
    assert!(said("slot 2 unreadable"));
    assert_eq!(
        store.stored(&Key {
            profile: P2,
            slot: 2
        }),
        None,
        "unknown profile"
    );
    assert_eq!(
        store.stored(&Key {
            profile: P1,
            slot: 8
        }),
        None,
        "past the slot count"
    );
}

#[test]
fn the_import_seeds_a_fresh_region_once() {
    let _h = host();
    let (store, enabled, level, _) = fixture!();
    store.open(Some(region()), 8);
    let payload = store.adapter().capture().payload;
    let body = |enabled| Body {
        enabled,
        payload: payload.clone(),
    };
    let records = [
        (
            Key {
                profile: P1,
                slot: 1,
            },
            body(false),
        ),
        (
            Key {
                profile: P1,
                slot: 2,
            },
            body(true),
        ),
        (
            Key {
                profile: P2,
                slot: 1,
            },
            body(false),
        ),
        (
            Key {
                profile: P2,
                slot: 8,
            },
            body(true),
        ), // past the count
        (
            Key {
                profile: P2,
                slot: 3,
            },
            Body {
                enabled: true,
                payload: vec![1],
            },
        ), // wrong size
    ];
    assert_eq!(store.import(&records), Ok(3));
    assert!(said("imported 3 record(s) over 2 save profile(s)"));
    let header = store.header().unwrap();
    assert_eq!(header.flags & HEADER_IMPORTED, HEADER_IMPORTED);
    assert_eq!(header.profiles, vec![P1, P2]);
    assert_eq!(store.mode(), Mode::Ready);
    assert!(FakeHost::flushes() >= 1);
    assert_eq!(store.import(&records), Err(status::STATE_CONFLICT), "once");
    level.set(6);
    store.on_event(&ev(LOADED, 2, P1, 1));
    assert_eq!((enabled.is_on(), level.get()), (true, 3));
    store.on_event(&ev(LOADED, 1, P2, 2));
    assert!(!enabled.is_on());
    // The header flag survives later saves.
    save(store, 5, P1, 3);
    let stored = FakeHost::take_region(ID).unwrap();
    assert_eq!(
        Header::parse(&stored.bytes).unwrap().flags & HEADER_IMPORTED,
        HEADER_IMPORTED
    );
}

#[test]
fn the_import_skips_profiles_past_capacity_and_nothing_to_import() {
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 2);
    assert_eq!(store.import(&[]), Ok(0));
    assert_eq!(store.mode(), Mode::Fresh);
    let payload = store.adapter().capture().payload;
    let records: Vec<(Key, Body)> = (0..20u8)
        .map(|p| {
            (
                Key {
                    profile: [p; 32],
                    slot: 0,
                },
                Body {
                    enabled: false,
                    payload: payload.clone(),
                },
            )
        })
        .collect();
    assert_eq!(store.import(&records), Ok(PROFILES));
    assert_eq!(store.header().unwrap().profiles.len(), PROFILES);
}

#[test]
fn a_large_import_goes_out_in_bounded_batches() {
    let _h = host();
    static WIDE: Raw = Raw {
        version: 2,
        bytes: 1000,
    };
    static STORE: PerSave = PerSave::new("test.unit", &WIDE);
    STORE.set_log(sink);
    assert_eq!(STORE.open(Some(region()), 200), Opened::Fresh);
    let records: Vec<(Key, Body)> = (0..3u8)
        .flat_map(|p| {
            (0..200).map(move |s| {
                (
                    Key {
                        profile: [p; 32],
                        slot: s,
                    },
                    WIDE.capture(),
                )
            })
        })
        .collect();
    // 3 × 200 × 1005 bytes: several 64 KiB batches (the fake host refuses larger).
    assert_eq!(STORE.import(&records), Ok(600));
    let region = FakeHost::regions()[0].clone();
    assert!(region.revision > 9, "{} batches", region.revision);
    assert_eq!(region.bytes.len() as u64, STORE.header().unwrap().length());
    let body = STORE
        .stored(&Key {
            profile: [2; 32],
            slot: 199,
        })
        .unwrap();
    assert_eq!(body.payload, vec![0x5a; 1000]);
    // Range-count bound: many small ranges split too.
    let tiny: Vec<(u64, &[u8])> = (0..600).map(|i| (i as u64, &b"x"[..])).collect();
    let groups = batches(&tiny);
    assert_eq!(
        groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
        vec![256, 256, 88]
    );
    assert!(batches(&[]).is_empty());
}

#[test]
fn a_failed_import_batch_stops_persisting() {
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 8);
    let payload = store.adapter().capture().payload;
    FakeHost::fail_services(0, status::STATE_IO_ERROR, 0, 0);
    let records = [(
        Key {
            profile: P1,
            slot: 0,
        },
        Body {
            enabled: false,
            payload,
        },
    )];
    assert_eq!(store.import(&records), Err(status::STATE_IO_ERROR));
    assert_eq!(store.mode(), Mode::Failed);
    assert!(said("import failed (-7)"));
    let _h = host();
    let (store, _, _, _) = fixture!();
    store.open(Some(region()), 8);
    let payload = store.adapter().capture().payload;
    FakeHost::fail_flush(status::STATE_IO_ERROR);
    let records = [(
        Key {
            profile: P1,
            slot: 0,
        },
        Body {
            enabled: false,
            payload,
        },
    )];
    assert_eq!(store.import(&records), Ok(1));
    assert!(said("flush after import failed (-7)"));
}

#[test]
fn deferred_saves_wait_for_the_writer_and_a_load_finishes_them_first() {
    let _h = host();
    let (store, enabled, _, _) = fixture!();
    store.open(Some(region()), 8);
    store.set_deferred(true);
    enabled.set(1);
    save(store, 1, P1, 1);
    assert_eq!(store.queued(), 1);
    assert_eq!(FakeHost::regions()[0].schema, 0, "not yet written");
    // A second save of the same slot replaces the queued one.
    enabled.set(0);
    save(store, 1, P1, 2);
    assert_eq!(store.queued(), 1);
    assert_eq!(store.write_queued(), 1);
    assert_eq!(store.queued(), 0);
    enabled.set(1);
    save(store, 2, P1, 3);
    assert_eq!(store.queued(), 1);
    store.on_event(&ev(LOADED, 2, P1, 4));
    assert_eq!(store.queued(), 0, "the load wrote the queue first");
    assert!(enabled.is_on());
    store.on_event(&ev(LOADED, 1, P1, 5));
    assert!(!enabled.is_on(), "the later capture won");
    assert_eq!(store.drain(), 0);
}

static WAKE_LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
fn wake_sink(_: u32, line: &str) {
    WAKE_LINES.lock().unwrap().push(line.to_owned());
}

#[test]
fn the_writer_thread_sleeps_until_a_save_is_queued() {
    // Memory-only store (no region): the writer drains and logs, on its own thread.
    static ENABLED: Param = Param::flag("Enabled", false);
    static PARAMS: Params = Params::new(1, &ENABLED, &[]);
    static STORE: PerSave = PerSave::new("test.writer", &PARAMS);
    STORE.set_log(wake_sink);
    STORE.open(None, 8);
    STORE.set_deferred(true);
    std::thread::spawn(|| -> () { STORE.run_writer() });
    std::thread::sleep(std::time::Duration::from_millis(5));
    save(&STORE, 0, P1, 1);
    for _ in 0..2000 {
        if STORE.queued() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(STORE.queued(), 0);
    for _ in 0..2000 {
        if WAKE_LINES
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("not persisted"))
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the writer never ran");
}

static CHANGES: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
fn changed() {
    CHANGES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

#[test]
fn the_params_adapter_packs_values_and_restores_through_set() {
    static ENABLED: Param = Param::flag("Enabled", false).with_on_change(changed);
    static A: Param = Param::new("A", 150, 10, 2000).with_on_change(changed);
    static B: Param = Param::flag("B", true);
    static PARAMS: Params = Params::new(3, &ENABLED, &[&A, &B]);
    assert!(core::ptr::eq(PARAMS.enabled(), &ENABLED));
    assert_eq!(PARAMS.params().len(), 2);
    assert_eq!(PARAMS.fields(), &[A.field(), B.field()]);
    assert!(
        core::ptr::eq(PARAMS.fields(), PARAMS.fields()),
        "built once"
    );
    assert_eq!(PARAMS.layout().version, 3);
    assert_eq!(PARAMS.payload_bytes(), 2, "11 + 1 bits");
    assert_eq!(PARAMS.version(), 3);
    assert_eq!(PARAMS.layout_hash(), layout_hash(3, PARAMS.fields()));
    ENABLED.set(1);
    A.set(777);
    B.set(0);
    let body = PARAMS.capture();
    assert!(body.enabled);
    assert_eq!(body.payload.len(), 2);
    PARAMS.defaults();
    assert_eq!((ENABLED.get(), A.get(), B.get()), (0, 150, 1));
    let before = CHANGES.load(core::sync::atomic::Ordering::Relaxed);
    PARAMS.restore(Some(&body));
    assert_eq!((ENABLED.get(), A.get(), B.get()), (1, 777, 0));
    assert!(
        CHANGES.load(core::sync::atomic::Ordering::Relaxed) >= before + 2,
        "callbacks ran"
    );
    // A payload of another layout reads as defaults, off.
    PARAMS.restore(Some(&Body {
        enabled: true,
        payload: vec![0xff, 0xff],
    }));
    assert_eq!((ENABLED.get(), A.get(), B.get()), (0, 150, 1));
    ENABLED.set(1);
    PARAMS.restore(Some(&Body {
        enabled: true,
        payload: vec![1],
    }));
    assert_eq!(ENABLED.get(), 0, "short payload");
    ENABLED.set(1);
    PARAMS.restore(None);
    assert_eq!(ENABLED.get(), 0);
    PARAMS.new_game();
}

#[test]
fn stores_and_adapters_also_build_at_run_time() {
    let _h = host();
    let enabled: &'static Param = Box::leak(Box::new(Param::flag("Enabled", false)));
    let params: &'static Params = Box::leak(Box::new(Params::new(1, enabled, &[])));
    let store = PerSave::new("run.time", params);
    assert_eq!(store.region_id().as_deref(), Some("run.time.per_save"));
    assert_eq!(params.payload_bytes(), 0);
    // The fake host enforces the host's batch rules.
    let r = region();
    let big = vec![0u8; abi::STATE_MAX_BATCH_BYTES + 1];
    assert_eq!(
        r.write_ranges(0, big.len() as u64, 1, &[(0, &big)]),
        Err(status::STATE_LIMIT)
    );
    let overlap: [(u64, &[u8]); 2] = [(0, b"abcd"), (2, b"ef")];
    assert_eq!(
        r.write_ranges(0, 8, 1, &overlap),
        Err(status::INVALID_ARGUMENT)
    );
    FakeHost::edit_region("missing", |_| unreachable!());
    assert!(FakeHost::providers().is_empty());
}

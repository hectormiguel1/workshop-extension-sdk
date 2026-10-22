//! Steps of `per_save.feature`: one store over three params, reopened on a
//! fresh fake host per scenario.
use super::Sdk;
use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicUsize, Ordering};
use cucumber::{given, then, when};
use workshop_ext_sdk::abi::{self, GAME_XIII, SaveEvent, status};
use workshop_ext_sdk::packed::Field;
use workshop_ext_sdk::param::Param;
use workshop_ext_sdk::per_save::{self, *};
use workshop_ext_sdk::settings::{Builder, Meta, controls};
use workshop_ext_sdk::storage::Region;
use workshop_ext_sdk::testing::{self, FakeHost};

const ID: &str = "bdd.unit.per_save";

static CHANGES: AtomicUsize = AtomicUsize::new(0);
fn changed() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

static ENABLED: Param = Param::flag("Enabled", false).with_on_change(changed);
static LEVEL: Param = Param::new("Level", 3, 1, 9);
static GIL: Param = Param::new("Gil", 100, 0, 1000);
static PARAMS: Params = Params::new(1, &ENABLED, &[&LEVEL, &GIL]);
static STORE: PerSave = PerSave::new("bdd.unit", &PARAMS);

thread_local! {
    static REGION: Cell<Option<Region>> = const { Cell::new(None) };
    static OPENED: Cell<Option<Opened>> = const { Cell::new(None) };
    static ANSWER: Cell<i32> = const { Cell::new(0) };
    static IMPORTED: Cell<Option<Result<usize, i32>>> = const { Cell::new(None) };
    static LINES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static SNAPSHOT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn sink(_: u32, line: &str) {
    LINES.with(|l| l.borrow_mut().push(line.to_owned()));
}

fn region() -> Region {
    REGION.with(|r| {
        let region = r
            .get()
            .unwrap_or_else(|| unsafe { Region::register_raw(&testing::STATE_API, ID) }.unwrap());
        r.set(Some(region));
        region
    })
}

fn fresh_session(w: &mut Sdk) {
    w.host = Some(Box::new(FakeHost::new(GAME_XIII)));
    REGION.with(|r| r.set(None));
    LINES.with(|l| l.borrow_mut().clear());
    PARAMS.defaults();
    STORE.set_deferred(false);
    STORE.set_log(sink);
}

fn profile(n: u8) -> [u8; 32] {
    [n; 32]
}

fn deliver(e: SaveEvent) {
    ANSWER.with(|a| a.set(STORE.on_event(&e)));
}

fn ev(kind: u32, slot: u32, p: u8, op: u64) -> SaveEvent {
    testing::event(kind, Some(slot), Some(profile(p)), op)
}

fn save(slot: u32, p: u8, op: u64) {
    deliver(ev(abi::SAVE_BEGIN, slot, p, op));
    deliver(ev(abi::SAVED, slot, p, op));
}

fn on(word: &str) -> bool {
    match word {
        "on" => true,
        "off" => false,
        other => panic!("on/off, not {other}"),
    }
}

fn reseal(b: &mut [u8]) {
    let c = crc32(&[&b[..28]]);
    b[28..32].copy_from_slice(&c.to_le_bytes());
}

#[given(expr = "a per-save store over {int} slots")]
fn store_over(w: &mut Sdk, slots: u32) {
    fresh_session(w);
    OPENED.with(|o| o.set(Some(STORE.open(Some(region()), slots))));
}

#[given("a per-save store without host storage")]
fn store_without(w: &mut Sdk) {
    fresh_session(w);
    OPENED.with(|o| o.set(Some(STORE.open(None, 8))));
}

#[then(expr = "a per-save store over {int} slots is refused")]
fn store_refused(_: &mut Sdk, slots: u32) {
    assert_eq!(
        STORE.open(Some(region()), slots),
        Opened::Foreign("unsupported slot count")
    );
    assert_eq!(
        STORE.open(Some(region()), MAX_SLOTS + 1),
        Opened::Foreign("unsupported slot count")
    );
}

#[then("a host that cannot describe the region fails the store")]
fn store_undescribed(w: &mut Sdk) {
    fresh_session(w);
    STORE.open(Some(region()), 8);
    save(0, 1, 1);
    FakeHost::info_size(4);
    assert_eq!(
        STORE.open(Some(region()), 8),
        Opened::Failed(status::DECODE_FAILED)
    );
    assert_eq!(STORE.mode(), Mode::Failed);
    assert!(said("region unreadable (-4)"));
}

#[then(expr = "the per-save store opened {word}")]
fn opened(_: &mut Sdk, what: String) {
    let o = OPENED.with(Cell::get).expect("opened");
    let ok = match what.as_str() {
        "fresh" => o == Opened::Fresh,
        "stored" => matches!(o, Opened::Stored(1)),
        "foreign" => matches!(o, Opened::Foreign(_)),
        "without-storage" => o == Opened::NoService,
        other => panic!("unknown opening {other}"),
    };
    assert!(ok, "{o:?}");
}

#[then(expr = "the per-save store is {word}")]
fn mode_is(_: &mut Sdk, what: String) {
    let want = match what.as_str() {
        "memory" => Mode::Memory,
        "fresh" => Mode::Fresh,
        "ready" => Mode::Ready,
        "foreign" => Mode::Foreign,
        "failed" => Mode::Failed,
        other => panic!("unknown mode {other}"),
    };
    assert_eq!(STORE.mode(), want);
}

#[when(expr = "the menu sets Enabled {word} and Level {int}")]
fn menu_sets(_: &mut Sdk, enabled: String, level: u32) {
    ENABLED.set(u32::from(on(&enabled)));
    LEVEL.set(level);
}

#[then(expr = "the per-save settings are Enabled {word}, Level {int}")]
fn settings_are(_: &mut Sdk, enabled: String, level: u32) {
    assert_eq!((ENABLED.is_on(), LEVEL.get()), (on(&enabled), level));
}

#[then("the per-save region is unwritten")]
fn unwritten(_: &mut Sdk) {
    let r = FakeHost::regions()
        .into_iter()
        .find(|r| r.id == ID)
        .unwrap();
    assert_eq!(r.schema, 0);
}

#[when(expr = "save operation {int} begins for slot {int} of profile {int}")]
fn begins(_: &mut Sdk, op: u64, slot: u32, p: u8) {
    deliver(ev(abi::SAVE_BEGIN, slot, p, op));
}

#[when(expr = "save operation {int} completes for slot {int} of profile {int}")]
fn completes(_: &mut Sdk, op: u64, slot: u32, p: u8) {
    deliver(ev(abi::SAVED, slot, p, op));
}

#[when(expr = "save operation {int} fails for slot {int} of profile {int}")]
fn fails(_: &mut Sdk, op: u64, slot: u32, p: u8) {
    deliver(ev(abi::SAVE_FAILED, slot, p, op));
}

#[when(expr = "slot {int} of profile {int} is saved as operation {int}")]
fn saved_as(_: &mut Sdk, slot: u32, p: u8, op: u64) {
    save(slot, p, op);
}

#[when(expr = "slot {int} of profile {int} is loaded")]
fn loaded(_: &mut Sdk, slot: u32, p: u8) {
    deliver(ev(abi::LOADED, slot, p, 9000));
}

#[when("a load arrives without a slot")]
fn load_no_slot(_: &mut Sdk) {
    deliver(testing::event(abi::LOADED, None, Some(profile(1)), 9001));
    assert_eq!(STORE.active(), None);
}

#[when(expr = "a load arrives without a profile for slot {int}")]
fn load_no_profile(_: &mut Sdk, slot: u32) {
    deliver(testing::event(abi::LOADED, Some(slot), None, 9002));
}

#[when("a save begins without a profile")]
fn begin_no_profile(_: &mut Sdk) {
    deliver(testing::event(abi::SAVE_BEGIN, Some(1), None, 9003));
}

#[when("a save begins without an operation id")]
fn begin_no_op(_: &mut Sdk) {
    deliver(ev(abi::SAVE_BEGIN, 1, 1, 0));
}

#[when(expr = "an event of unknown kind {int} arrives")]
fn unknown_kind(_: &mut Sdk, kind: u32) {
    let mut e = ev(abi::SAVED, 1, 1, 9004);
    e.kind = kind;
    deliver(e);
}

#[when("a new game starts")]
fn new_game(_: &mut Sdk) {
    deliver(testing::event(abi::NEW_GAME, None, Some(profile(1)), 9005));
    assert_eq!(STORE.active(), None);
}

#[when(expr = "{int} saves begin for slot {int} of profile {int}")]
fn many_begin(_: &mut Sdk, n: u64, slot: u32, p: u8) {
    for op in 1..=n {
        deliver(ev(abi::SAVE_BEGIN, slot, p, op));
        assert_eq!(ANSWER.with(Cell::get), status::OK);
    }
}

#[when(expr = "{int} more save profiles each save slot {int}")]
fn more_profiles(_: &mut Sdk, n: u8, slot: u32) {
    for p in 0..n {
        save(slot, 100 + p, 1000 + u64::from(p));
        assert_eq!(ANSWER.with(Cell::get), status::OK);
    }
    assert_eq!(STORE.header().unwrap().profiles.len(), PROFILES);
}

#[then(expr = "the per-save event answered {word}")]
fn answered(_: &mut Sdk, what: String) {
    let want = match what.as_str() {
        "ok" => status::OK,
        "not-found" => status::NOT_FOUND,
        "invalid" => status::INVALID_ARGUMENT,
        "limit" => status::STATE_LIMIT,
        other => panic!("unknown answer {other}"),
    };
    assert_eq!(ANSWER.with(Cell::get), want);
}

fn said(text: &str) -> bool {
    LINES.with(|l| l.borrow().iter().any(|line| line.contains(text)))
}

#[then(expr = "the per-save log mentions {string}")]
fn log_mentions(_: &mut Sdk, text: String) {
    assert!(said(&text), "{:?}", LINES.with(|l| l.borrow().clone()));
}

#[then(expr = "slot {int} of profile {int} is stored {word}")]
fn stored_as(_: &mut Sdk, slot: u32, p: u8, what: String) {
    let body = STORE.stored(&Key {
        profile: profile(p),
        slot,
    });
    match what.as_str() {
        "absent" => assert_eq!(body, None),
        word => assert_eq!(body.map(|b| b.enabled), Some(on(word))),
    }
}

#[then(expr = "{int} captures are pending")]
fn pending(_: &mut Sdk, n: usize) {
    assert_eq!(STORE.pending(), n);
}

#[then(expr = "{int} writes are queued")]
fn queued(_: &mut Sdk, n: usize) {
    assert_eq!(STORE.queued(), n);
}

#[when("another writer bumps the region revision")]
fn bump(_: &mut Sdk) {
    FakeHost::edit_region(ID, |r| r.revision += 5);
}

#[when("another writer changes the region header")]
fn change_header(_: &mut Sdk) {
    FakeHost::edit_region(ID, |r| {
        r.revision += 1;
        r.bytes[16] ^= 1;
    });
}

#[when(expr = "the host refuses storage writes with {int}")]
fn refuse_writes(_: &mut Sdk, rc: i32) {
    FakeHost::fail_services(0, rc, 0, 0);
}

#[when(expr = "the host flush fails with {int}")]
fn flush_fails(_: &mut Sdk, rc: i32) {
    FakeHost::fail_flush(rc);
}

#[when(expr = "the next session opens the stored region over {int} slots")]
fn next_session(w: &mut Sdk, slots: u32) {
    let stored = FakeHost::take_region(ID).unwrap();
    fresh_session(w);
    FakeHost::seed_region(ID, stored.schema, &stored.bytes);
    OPENED.with(|o| o.set(Some(STORE.open(Some(region()), slots))));
}

#[when(expr = "the next session finds the region damaged: {word}")]
fn damaged_session(w: &mut Sdk, damage: String) {
    let mut stored = FakeHost::take_region(ID).unwrap();
    let b = &mut stored.bytes;
    let mut schema = stored.schema;
    match damage.as_str() {
        "schema" => schema = 9,
        "short" => b.truncate(100),
        "magic" => b[0] = b'Z',
        "layout" => {
            b[12] ^= 1;
            reseal(b);
        }
        "slots" => {
            b[16] = 7;
            reseal(b);
        }
        "truncated" => b.truncate(RECORDS_AT + 3),
        other => panic!("unknown damage {other}"),
    }
    SNAPSHOT.with(|s| *s.borrow_mut() = stored.bytes.clone());
    fresh_session(w);
    FakeHost::seed_region(ID, schema, &stored.bytes);
    OPENED.with(|o| o.set(Some(STORE.open(Some(region()), 8))));
    assert!(said("region left untouched"));
}

#[then("the per-save region bytes are unchanged")]
fn region_unchanged(_: &mut Sdk) {
    let r = FakeHost::regions()
        .into_iter()
        .find(|r| r.id == ID)
        .unwrap();
    assert_eq!(r.revision, 1);
    SNAPSHOT.with(|s| assert_eq!(r.bytes, *s.borrow()));
}

#[when(expr = "the stored record of slot {int} is damaged")]
fn damage_record(_: &mut Sdk, slot: u32) {
    let at = STORE.header().unwrap().offset(0, slot) as usize;
    FakeHost::edit_region(ID, |r| r.bytes[at + 1] ^= 0xff);
}

#[when("the stored records are cut off")]
fn cut_records(_: &mut Sdk) {
    FakeHost::edit_region(ID, |r| r.bytes.truncate(RECORDS_AT));
}

#[when(expr = "legacy records are imported: {int} profiles with {int} slots each, Enabled {word}")]
fn import(_: &mut Sdk, profiles: u8, slots: u32, enabled: String) {
    LEVEL.set(6);
    let payload = PARAMS.capture().payload;
    PARAMS.defaults();
    let records: Vec<(Key, Body)> = (1..=profiles)
        .flat_map(|p| {
            let payload = payload.clone();
            let enabled = on(&enabled);
            (0..slots).map(move |slot| {
                (
                    Key {
                        profile: profile(p),
                        slot,
                    },
                    Body {
                        enabled,
                        payload: payload.clone(),
                    },
                )
            })
        })
        .collect();
    IMPORTED.with(|i| i.set(Some(STORE.import(&records))));
}

#[then(expr = "{int} records were imported")]
fn imported(_: &mut Sdk, n: usize) {
    assert_eq!(IMPORTED.with(Cell::get), Some(Ok(n)));
}

#[then("the per-save region is marked imported")]
fn marked(_: &mut Sdk) {
    assert_eq!(
        STORE.header().unwrap().flags & HEADER_IMPORTED,
        HEADER_IMPORTED
    );
}

#[then("the import is refused")]
fn import_refused(_: &mut Sdk) {
    assert_eq!(STORE.import(&[]), Err(status::STATE_CONFLICT));
}

/// A raw adapter with large records.
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

#[then(expr = "a wide import over {int} slots of {int} profiles writes several 64 KiB batches")]
fn wide_import(w: &mut Sdk, slots: u32, profiles: u8) {
    static WIDE: Raw = Raw {
        version: 2,
        bytes: 1000,
    };
    static WIDE_STORE: PerSave = PerSave::new("bdd.unit", &WIDE);
    fresh_session(w);
    WIDE_STORE.set_log(sink);
    assert_eq!(WIDE_STORE.open(Some(region()), slots), Opened::Fresh);
    let records: Vec<(Key, Body)> = (0..profiles)
        .flat_map(|p| {
            (0..slots).map(move |s| {
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
    assert_eq!(WIDE_STORE.import(&records), Ok(records.len()));
    let r = FakeHost::regions()
        .into_iter()
        .find(|r| r.id == ID)
        .unwrap();
    assert!(r.revision > 9, "{} batches", r.revision);
    let last = Key {
        profile: [profiles - 1; 32],
        slot: slots - 1,
    };
    assert_eq!(WIDE_STORE.stored(&last).unwrap().payload, vec![0x5a; 1000]);
    // The rest of the scenario uses the params store again.
    fresh_session(w);
    STORE.open(Some(region()), 8);
}

#[then("records larger than the contract are refused")]
fn too_large(_: &mut Sdk) {
    static BIG: Raw = Raw {
        version: 1,
        bytes: MAX_PAYLOAD + 1,
    };
    static OLD: Raw = Raw {
        version: 64,
        bytes: 4,
    };
    static BIG_STORE: PerSave = PerSave::new("bdd.big", &BIG);
    static OLD_STORE: PerSave = PerSave::new("bdd.old", &OLD);
    assert_eq!(
        BIG_STORE.open(Some(region()), 8),
        Opened::Foreign("unsupported record")
    );
    assert_eq!(
        OLD_STORE.open(Some(region()), 8),
        Opened::Foreign("unsupported record")
    );
    assert_eq!(OLD_STORE.unit(), "bdd.old");
    assert_eq!(OLD_STORE.region_id().as_deref(), Some("bdd.old.per_save"));
    assert_eq!(OLD_STORE.adapter().version(), 64);
    BIG.new_game();
}

#[when("the store defers writes to its writer")]
fn defer(_: &mut Sdk) {
    STORE.set_deferred(true);
}

#[when("the writer writes the queue")]
fn writer(_: &mut Sdk) {
    assert_eq!(STORE.write_queued(), 1);
    assert_eq!(STORE.drain(), 0);
}

#[then("a record of slot 4 decodes for slot 4 only and at its own version")]
fn record_identity(_: &mut Sdk) {
    let key = Key {
        profile: profile(1),
        slot: 4,
    };
    let body = Body {
        enabled: true,
        payload: vec![9, 8, 7],
    };
    let bytes = encode_record(&key, 5, &body);
    assert_eq!(bytes.len(), record_bytes(3));
    assert_eq!(decode_record(&key, 5, &bytes), Ok(Some(body)));
    let other = Key {
        profile: profile(1),
        slot: 5,
    };
    assert_eq!(decode_record(&other, 5, &bytes), Err(RecordError::Checksum));
    assert_eq!(decode_record(&key, 6, &bytes), Err(RecordError::Version(5)));
    assert_eq!(
        encode_record(
            &key,
            200,
            &Body {
                enabled: false,
                payload: vec![]
            }
        )[0] >> VERSION_SHIFT,
        MAX_VERSION
    );
    let good = ev(abi::SAVED, 3, 1, 1);
    assert_eq!(
        Key::of(&good, 99),
        Some(Key {
            profile: profile(1),
            slot: 3
        })
    );
    assert_eq!(Key::of(&good, 3), None);
}

#[then("a never-saved record reads as no record and a short one is refused")]
fn record_empty(_: &mut Sdk) {
    let key = Key {
        profile: profile(1),
        slot: 0,
    };
    assert_eq!(decode_record(&key, 1, &[0; 8]), Ok(None));
    assert_eq!(decode_record(&key, 1, &[1, 2]), Err(RecordError::Length));
}

#[then("a header round-trips and rejects a bad magic, checksum, version and directory")]
fn header_identity(_: &mut Sdk) {
    let mut header = Header::new(99, 24, 0xdead_beef);
    header.profiles = vec![profile(1)];
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(&profile(1));
    bytes.resize(RECORDS_AT, 0);
    assert_eq!(Header::parse(&bytes), Ok(header.clone()));
    assert_eq!(header.length(), (RECORDS_AT + 99 * 24) as u64);
    assert_eq!(
        Header::parse(&bytes[..10]),
        Err("region shorter than its header")
    );
    let mut b = bytes.clone();
    b[0] = 0;
    assert_eq!(Header::parse(&b), Err("not a per-save region"));
    let mut b = bytes.clone();
    b[16] ^= 1;
    assert_eq!(Header::parse(&b), Err("header checksum mismatch"));
    let mut b = bytes.clone();
    b[8] = 3;
    reseal(&mut b);
    assert_eq!(Header::parse(&b), Err("unknown container version"));
    let mut b = bytes;
    b[22] = 17;
    reseal(&mut b);
    assert_eq!(Header::parse(&b), Err("bad profile directory"));
    assert_eq!(per_save::region_id("bad id"), None);
}

#[then("layout hashes follow keys and ranges, not defaults")]
fn layout_hashes(_: &mut Sdk) {
    let a = [Field::new("Level", 3, 1, 9)];
    let b = [Field::new("LEVEL", 5, 1, 9)];
    let c = [Field::new("Level", 3, 1, 10)];
    assert_eq!(layout_hash(1, &a), layout_hash(1, &b));
    assert_ne!(layout_hash(1, &a), layout_hash(1, &c));
    assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926);
    assert_eq!(fnv1a(b"a"), 0xe40c_292c);
}

#[then(
    expr = "the params adapter packs Enabled on, Level {int}, Gil {int} into {int} bytes and back"
)]
fn params_pack(_: &mut Sdk, level: u32, gil: u32, bytes: usize) {
    assert!(core::ptr::eq(PARAMS.enabled(), &ENABLED));
    assert_eq!(PARAMS.params().len(), 2);
    assert_eq!(PARAMS.fields(), &[LEVEL.field(), GIL.field()]);
    assert_eq!(PARAMS.layout().schema, 1);
    assert_eq!(PARAMS.layout_hash(), layout_hash(1, PARAMS.fields()));
    ENABLED.set(1);
    LEVEL.set(level);
    GIL.set(gil);
    let body = PARAMS.capture();
    assert_eq!(
        (body.enabled, body.payload.len(), PARAMS.payload_bytes()),
        (true, bytes, bytes)
    );
    PARAMS.defaults();
    let before = CHANGES.load(Ordering::Relaxed);
    PARAMS.restore(Some(&body));
    assert_eq!(
        (ENABLED.is_on(), LEVEL.get(), GIL.get()),
        (true, level, gil)
    );
    assert!(
        CHANGES.load(Ordering::Relaxed) > before,
        "the Enabled callback ran"
    );
    PARAMS.new_game();
}

#[then("a payload of another layout restores defaults, off")]
fn params_foreign(_: &mut Sdk) {
    ENABLED.set(1);
    PARAMS.restore(Some(&Body {
        enabled: true,
        payload: vec![0xff, 0xff],
    }));
    assert_eq!((ENABLED.is_on(), LEVEL.get(), GIL.get()), (false, 3, 100));
    ENABLED.set(1);
    PARAMS.restore(None);
    assert!(!ENABLED.is_on());
}

#[then(expr = "a param over 10..2000 clamps 5000 to 2000 and reports the change once")]
fn param_clamps(_: &mut Sdk) {
    let hp = Param::new("Hp", 150, 10, 2000).with_on_change(changed);
    let before = CHANGES.load(Ordering::Relaxed);
    assert!(hp.set(5000));
    assert_eq!(hp.get(), 2000);
    assert!(!hp.set(2000));
    assert!(hp.reset());
    assert_eq!(hp.get(), 150);
    assert_eq!(CHANGES.load(Ordering::Relaxed), before + 2);
    assert_eq!(hp.field(), Field::new("Hp", 150, 10, 2000));
    assert!(Param::flag("F", true).is_on());
}

#[then("menu values round half up, negatives read 0 and NaN is ignored")]
fn menu_values(_: &mut Sdk) {
    assert_eq!(Param::from_menu(f32::NAN), None);
    assert_eq!(Param::from_menu(f32::NEG_INFINITY), None);
    assert_eq!(Param::from_menu(-2.0), Some(0));
    assert_eq!(Param::from_menu(2.49), Some(2));
    assert_eq!(Param::from_menu(2.5), Some(3));
}

#[then("param controls write their params through the menu callbacks")]
fn param_controls(_: &mut Sdk) {
    static ON: Param = Param::flag("On", false);
    static HP: Param = Param::new("Hp", 150, 10, 2000);
    static MODE: Param = Param::new("Mode", 0, 0, 2);
    let p = Builder::new()
        .param_checkbox("on", "On", "", &ON)
        .param_integer("hp", "HP", "", 10, &HP)
        .param_combo("mode", "Mode", "", &["A", "B", "C"], &MODE)
        .finish(&Meta {
            id: "bdd.params",
            name: "Params",
            author: "Tests",
            release: "1",
            help: "",
            website: None,
        });
    let c = unsafe { controls(p) };
    assert_eq!(c.len(), 3);
    assert_eq!((c[1].min, c[1].max, c[1].step), (10.0, 2000.0, 10.0));
    unsafe {
        (c[0].write.unwrap())(c[0].user, 1.0);
        (c[1].write.unwrap())(c[1].user, 777.4);
        (c[1].write.unwrap())(c[1].user, f32::NAN);
        (c[2].write.unwrap())(c[2].user, 9.0);
        assert_eq!((c[1].read.unwrap())(c[1].user), 777.0);
    }
    assert_eq!((ON.is_on(), HP.get(), MODE.get()), (true, 777, 2));
}

fn payload() -> Vec<u8> {
    PARAMS.capture().payload
}

#[then("an import skips slots past the count, other payload sizes and profiles past capacity")]
fn import_skips(w: &mut Sdk) {
    fresh_session(w);
    STORE.open(Some(region()), 2);
    let body = |payload| Body {
        enabled: false,
        payload,
    };
    let mut records = vec![
        (
            Key {
                profile: profile(1),
                slot: 2,
            },
            body(payload()),
        ),
        (
            Key {
                profile: profile(1),
                slot: 0,
            },
            body(vec![1]),
        ),
    ];
    records.extend((0..20u8).map(|p| {
        (
            Key {
                profile: [p + 30; 32],
                slot: 1,
            },
            body(payload()),
        )
    }));
    assert_eq!(STORE.import(&records), Ok(PROFILES));
    assert_eq!(STORE.header().unwrap().profiles.len(), PROFILES);
    assert_eq!(
        STORE.stored(&Key {
            profile: profile(1),
            slot: 0
        }),
        None,
        "unknown profile"
    );
}

#[then("an import of nothing writes nothing")]
fn import_nothing(w: &mut Sdk) {
    fresh_session(w);
    STORE.open(Some(region()), 2);
    assert_eq!(STORE.import(&[]), Ok(0));
    assert_eq!(STORE.mode(), Mode::Fresh);
}

#[then("a flush failure after an import is logged")]
fn import_flush(w: &mut Sdk) {
    fresh_session(w);
    STORE.open(Some(region()), 2);
    FakeHost::fail_flush(status::STATE_IO_ERROR);
    let records = [(
        Key {
            profile: profile(1),
            slot: 0,
        },
        Body {
            enabled: true,
            payload: payload(),
        },
    )];
    assert_eq!(STORE.import(&records), Ok(1));
    assert!(said("flush after import failed (-7)"));
}

#[then("stores and adapters also build at run time")]
fn run_time(_: &mut Sdk) {
    let enabled: &'static Param = Box::leak(Box::new(Param::flag("Enabled", false)));
    let params: &'static Params = Box::leak(Box::new(Params::new(1, enabled, &[])));
    let store = PerSave::new("run.time", params);
    assert_eq!(store.region_id().as_deref(), Some("run.time.per_save"));
    assert_eq!(params.payload_bytes(), 0);
}

static WRITER_LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
fn writer_sink(_: u32, line: &str) {
    WRITER_LINES.lock().unwrap().push(line.to_owned());
}

#[then("a writer thread handles a deferred save of a store without storage")]
fn writer_thread(_: &mut Sdk) {
    static ON: Param = Param::flag("Enabled", false);
    static P: Params = Params::new(1, &ON, &[]);
    static WRITER: PerSave = PerSave::new("bdd.writer", &P);
    WRITER.set_log(writer_sink);
    WRITER.open(None, 8);
    WRITER.set_deferred(true);
    std::thread::spawn(|| -> () { WRITER.run_writer() });
    std::thread::sleep(std::time::Duration::from_millis(5));
    WRITER.on_event(&ev(abi::SAVE_BEGIN, 0, 1, 1));
    WRITER.on_event(&ev(abi::SAVED, 0, 1, 1));
    for _ in 0..2000 {
        if WRITER_LINES
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("not persisted"))
        {
            assert_eq!(WRITER.queued(), 0);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the writer never ran");
}

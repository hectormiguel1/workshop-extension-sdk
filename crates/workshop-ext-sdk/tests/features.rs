//! Cucumber suite for the SDK: the host boundary, the shared site policy,
//! hook installs, storage, settings, save events, packed records and the
//! unit exports, all over the in-memory host of `workshop_ext_sdk::testing`.
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use cucumber::{World, given, then, when, writer::Stats as _};
use std::borrow::Cow;
use std::sync::Mutex;
use workshop_ext_sdk::abi::{
    self, GAME_XIII, HostContext, SaveEvent, SaveEventInfo, SaveEventsApi, SettingControl,
    SettingDisplayExtension, SettingPresentationExtension, SettingStateExtension,
    SettingTextExtension, SettingsApi, SettingsProvider, SettingsQueryContext, StateApi, StateInfo,
    level, status,
};
use workshop_ext_sdk::hooks::{HookFailure, HookSpec, HookStage};
use workshop_ext_sdk::host::{
    Host, HostSlot, ReadError, Requirements, Service, ServiceError, Unusable,
};
use workshop_ext_sdk::image::Image;
use workshop_ext_sdk::packed::{self, Field, Layout, PackedRecord, PackedValue, TextEntry};
use workshop_ext_sdk::save_events::{self, EventKind, HANDLER_PANICKED};
use workshop_ext_sdk::settings::{self, Builder, Meta};
use workshop_ext_sdk::sites::{
    self, Finding, Jump, NoForeignCode, Problem, Site, SiteKind, Summary, Verdict,
};
use workshop_ext_sdk::storage::{self, Region, RegionInfo};
use workshop_ext_sdk::testing::{self, FakeHost, FakeMemory, save_event};
use workshop_ext_sdk::unit::{self, Init, InitError, InitResult, StorageError, code};

#[path = "steps/per_save.rs"]
mod per_save_steps;
#[path = "steps/texts.rs"]
mod texts_steps;

#[derive(Default, World)]
struct Sdk {
    host: Option<Box<FakeHost>>,
    image: Vec<u8>,
    findings: Vec<(&'static str, Finding)>,
    summary: Summary,
    failure: Option<HookFailure>,
    region: Option<Region>,
    provider: Option<&'static SettingsProvider>,
    service: Option<ServiceError>,
}
// Keep the Cucumber World small: one is created for every scenario.
const _: () = assert!(
    size_of::<Sdk>() <= 8192,
    "the Cucumber World is above 8 KiB: box its large fields"
);

impl std::fmt::Debug for Sdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sdk")
            .field("findings", &self.findings)
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}

impl Sdk {
    fn fake(&mut self) -> &mut FakeHost {
        self.host.as_mut().expect("a fake host")
    }
    fn context(&self) -> *const HostContext {
        self.host.as_ref().expect("a fake host").context()
    }
    fn live(&self) -> Host {
        unsafe { Host::from_raw(self.context()) }.unwrap()
    }
}

// ---- host ----------------------------------------------------------------

#[given("a fake FINAL FANTASY XIII host")]
#[when("a fake FINAL FANTASY XIII host registers the declared provider")]
fn fake_host(w: &mut Sdk) {
    w.host = Some(Box::new(FakeHost::new(GAME_XIII)));
}

unsafe extern "C" fn short_read(
    _: *const u8,
    _: usize,
    _: *mut u8,
    _: usize,
    w: *mut usize,
) -> i32 {
    unsafe { w.write(0) };
    status::SHORT_BUFFER
}

#[when(expr = "the context's {word} is {word}")]
fn break_context(w: &mut Sdk, field: String, value: String) {
    if field == "pointer" {
        w.host = None;
        return;
    }
    let number = value.parse::<usize>().unwrap_or(0);
    let c = w.fake().context_mut();
    match field.as_str() {
        "version" => c.version = number as u32,
        "size" if value == "prefix" => c.size = abi::prefix_size() as u32,
        "size" => c.size = number as u32,
        "game" => c.game = number as u32,
        "game_base" => c.game_base = number,
        "vfs_read" => c.vfs_read = None,
        "log" => c.log = None,
        "hook_create" => c.hook_create = None,
        other => panic!("unknown field {other}"),
    }
}

#[then(expr = "the context is refused with {string}")]
fn refused(w: &mut Sdk, reason: String) {
    let ptr = w.host.as_ref().map_or(core::ptr::null(), |h| h.context());
    let why = unsafe { Host::from_raw(ptr) }.unwrap_err();
    assert_eq!(why.to_string(), reason);
    assert_eq!(
        unsafe { unit::run_init(ptr, |_| Ok(())) },
        code::UNUSABLE_CONTEXT
    );
}

const XIII_ALL: Requirements = Requirements {
    game: Some(GAME_XIII),
    services: &[Service::Log, Service::VfsSize, Service::VfsRead],
    image: true,
};

#[then(expr = "requiring FINAL FANTASY XIII with log and VFS and an image fails with {string}")]
fn require_fails(w: &mut Sdk, reason: String) {
    assert_eq!(w.live().require(&XIII_ALL).unwrap_err().to_string(), reason);
}

#[then("every requirement is met")]
fn requirements_met(w: &mut Sdk) {
    let host = w.live();
    assert_eq!(host.require(&XIII_ALL), Ok(()));
    assert_eq!(host.require(&Requirements::ANY), Ok(()));
    let hooks = Requirements {
        services: &Service::HOOKS,
        ..Requirements::ANY
    };
    assert_eq!(host.require(&hooks), Ok(()));
    let mut c = unsafe { *w.context() }.prefix();
    let other = abi::HostContextV1 { version: 2, ..c };
    assert_eq!(other.require(&Requirements::ANY), Err(Unusable::Version(2)));
    let short = abi::HostContextV1 { size: 8, ..c };
    assert_eq!(short.require(&Requirements::ANY), Err(Unusable::Size(8)));
    c.game_size = 0;
    assert_eq!(c.require(&XIII_ALL), Err(Unusable::Image));
    c.hook_disable = None;
    assert_eq!(
        c.require(&hooks),
        Err(Unusable::MissingService("hook_disable"))
    );
    c.hook_enable = None;
    assert_eq!(
        c.require(&hooks),
        Err(Unusable::MissingService("hook_enable"))
    );
    c.hook_remove = None;
    c.hook_create = None;
    assert_eq!(
        c.require(&hooks),
        Err(Unusable::MissingService("hook_create"))
    );
    let only_remove = Requirements {
        services: &[Service::HookRemove],
        ..Requirements::ANY
    };
    assert_eq!(
        c.require(&only_remove),
        Err(Unusable::MissingService("hook_remove"))
    );
    c.vfs_size = None;
    assert_eq!(
        c.require(&XIII_ALL),
        Err(Unusable::MissingService("vfs_size"))
    );
}

#[then("every service names its context field")]
fn service_names(_: &mut Sdk) {
    let names: Vec<&str> = [
        Service::Log,
        Service::VfsSize,
        Service::VfsRead,
        Service::HookCreate,
        Service::HookEnable,
        Service::HookDisable,
        Service::HookRemove,
    ]
    .map(Service::name)
    .to_vec();
    assert_eq!(
        names,
        [
            "log",
            "vfs_size",
            "vfs_read",
            "hook_create",
            "hook_enable",
            "hook_disable",
            "hook_remove"
        ]
    );
    assert_eq!(
        abi::game_name(abi::GAME_LR),
        "LIGHTNING RETURNS: FINAL FANTASY XIII"
    );
    assert_eq!(abi::game_name(abi::GAME_XIII_2), "FINAL FANTASY XIII-2");
    assert_eq!(abi::game_name(0), "unknown game");
}

#[given(expr = "the merged containers hold {string} with {string}")]
fn add_file(_: &mut Sdk, path: String, content: String) {
    FakeHost::add_file(&path, content.as_bytes());
}

#[then(expr = "reading {string} with a limit of {int} bytes gives {string}")]
fn read_ok(w: &mut Sdk, path: String, max: u64, content: String) {
    assert_eq!(w.live().read_file(&path, max).unwrap(), content.as_bytes());
    assert_eq!(w.live().file_size(&path), Ok(content.len() as u64));
}

#[then(
    expr = "reading {string} with a limit of {int} bytes is refused as too large \\({int} bytes)"
)]
fn read_large(w: &mut Sdk, path: String, max: u64, size: u64) {
    assert_eq!(
        w.live().read_file(&path, max),
        Err(ReadError::TooLarge(size))
    );
}

#[then(expr = "reading {string} reports host status {int}")]
fn read_status(w: &mut Sdk, path: String, rc: i32) {
    assert_eq!(w.live().read_file(&path, 64), Err(ReadError::Status(rc)));
}

#[then(expr = "a host whose read fails reports host status {int}")]
fn read_fails(w: &mut Sdk, rc: i32) {
    let mut c = unsafe { *w.context() }.prefix();
    c.vfs_read = Some(short_read);
    assert_eq!(c.read_file("vfx/x.xfv", 64), Err(ReadError::Status(rc)));
}

#[then("a host without VFS services reports the read unavailable")]
fn read_unavailable(w: &mut Sdk) {
    let mut c = unsafe { *w.context() }.prefix();
    c.vfs_size = None;
    assert_eq!(c.read_file("a", 1), Err(ReadError::Unavailable));
    assert_eq!(c.file_size("a"), Err(ReadError::Unavailable));
}

#[when("the unit logs at every level")]
fn log_levels(w: &mut Sdk) {
    let host = w.live();
    host.log(level::INFO, "plain");
    host.error(format_args!("e {}", 1));
    host.warn(format_args!("w {}", 2));
    host.info(format_args!("i {}", 3));
}

#[then(expr = "the host received the lines {string}, {string}, {string} and {string}")]
fn received(_: &mut Sdk, a: String, b: String, c: String, d: String) {
    assert_eq!(
        FakeHost::lines(),
        [
            (level::INFO, a),
            (level::ERROR, b),
            (level::WARN, c),
            (level::INFO, d)
        ]
    );
}

#[then("a context without a log drops lines silently")]
fn no_log(w: &mut Sdk) {
    let mut c = unsafe { *w.context() }.prefix();
    c.log = None;
    c.log_line(level::DEBUG, b"dropped");
    assert!(!FakeHost::logged("dropped"));
}

#[then("storage, settings and save events are all advertised")]
fn all_advertised(w: &mut Sdk) {
    let h = w.live();
    assert!(h.storage_api().unwrap().is_some());
    assert!(h.settings_api().unwrap().is_some());
    assert!(h.save_events_api().unwrap().is_some());
    assert!(h.hooks().is_some());
    assert_eq!(
        (h.game(), h.phase(), h.raw()),
        (GAME_XIII, abi::PHASE_LATE, w.context())
    );
    assert_eq!(
        h.image(),
        Image {
            base: 0x40_0000,
            size: 0x251_9000
        }
    );
}

#[then("storage, settings and save events are all absent")]
fn all_absent(w: &mut Sdk) {
    let h = w.live();
    assert!(h.storage_api().unwrap().is_none());
    assert!(h.settings_api().unwrap().is_none());
    assert!(h.save_events_api().unwrap().is_none());
}

static BAD_STATE: [StateApi; 3] = [
    StateApi {
        read_at: None,
        ..testing::STATE_API
    },
    StateApi {
        version: 2,
        ..testing::STATE_API
    },
    StateApi {
        size: 8,
        ..testing::STATE_API
    },
];
static BAD_SETTINGS: [SettingsApi; 3] = [
    SettingsApi {
        register_provider: None,
        ..testing::SETTINGS_API
    },
    SettingsApi {
        version: 0,
        ..testing::SETTINGS_API
    },
    SettingsApi {
        size: 4,
        ..testing::SETTINGS_API
    },
];
static BAD_SAVES: [SaveEventsApi; 3] = [
    SaveEventsApi {
        register_callback: None,
        ..testing::SAVE_EVENTS_API
    },
    SaveEventsApi {
        version: 7,
        ..testing::SAVE_EVENTS_API
    },
    SaveEventsApi {
        size: 4,
        ..testing::SAVE_EVENTS_API
    },
];

#[when(regex = r"^the host advertises a (storage|settings|save events) table with (.+)$")]
fn advertise_bad(w: &mut Sdk, table: String, defect: String) {
    let index = settings_index(&defect);
    let c = w.fake().context_mut();
    match table.as_str() {
        "storage" => c.storage = Some(&BAD_STATE[index]),
        "settings" => c.settings = Some(&BAD_SETTINGS[index]),
        _ => c.save_events = Some(&BAD_SAVES[index]),
    }
    let h = w.live();
    w.service = h
        .storage_api()
        .err()
        .or_else(|| h.settings_api().err())
        .or_else(|| h.save_events_api().err());
}

/// Index into the malformed tables: [missing entry, version, size].
fn settings_index(defect: &str) -> usize {
    match defect.split(' ').next() {
        Some("no") => 0,
        Some("version") => 1,
        _ => 2,
    }
}

#[then(expr = "the service is refused with {string}")]
fn service_refused(w: &mut Sdk, reason: String) {
    let error = w.service.expect("a malformed table");
    assert_eq!(error.to_string(), reason);
    assert_eq!(InitError::from(error), InitError::REFUSED);
}

#[then("a host slot remembers the context until cleared")]
fn slot(w: &mut Sdk) {
    let slot = HostSlot::default();
    assert!(slot.get().is_none());
    slot.set(&w.live());
    assert_eq!(slot.get().map(|h| h.game()), Some(GAME_XIII));
    slot.clear();
    assert!(slot.get().is_none() && HostSlot::new().get().is_none());
}

const HOST_HEADER: &str = include_str!("../include/ff13_host.h");

#[then("the x86 context offsets match the header's static assertions")]
fn header_layout(_: &mut Sdk) {
    let x86 = abi::context_offsets(4);
    for (field, index) in [("game_base", 4), ("log", 6), ("hook_remove", 12)] {
        let assert = format!(
            "_Static_assert(offsetof(FF13HostContext, {field}) == {}",
            x86[index]
        );
        assert!(HOST_HEADER.contains(&assert), "{assert}");
    }
    let size = format!(
        "_Static_assert(sizeof(FF13HostContext) == {}",
        abi::CONTEXT_SIZE_X86
    );
    assert!(HOST_HEADER.contains(&size));
    assert_eq!(x86[13] as u32, abi::PREFIX_SIZE_X86);
    assert_eq!(&x86[..13], abi::prefix_offsets(4));
    assert_eq!(
        abi::context_offsets(size_of::<usize>())[13],
        abi::prefix_size()
    );
    assert!(HOST_HEADER.contains("#define FF13_HOST_ABI_VERSION 1u"));
}

#[then("a prefix-sized context reads its service tables as absent")]
fn prefix_read(w: &mut Sdk) {
    assert!(unsafe { HostContext::read(core::ptr::null()) }.is_none());
    w.fake().context_mut().size = abi::prefix_size() as u32;
    let copy = unsafe { HostContext::read(w.context()) }.unwrap();
    assert!(copy.storage.is_none() && copy.settings.is_none() && copy.save_events.is_none());
    assert!(copy.covers(abi::prefix_size()) && !copy.covers(abi::prefix_size() + 1));
    let back = HostContext::from_prefix(copy.prefix());
    assert_eq!(
        (back.game, back.game_base, back.storage.is_none()),
        (GAME_XIII, 0x40_0000, true)
    );
}

#[then(expr = "a default line keeps {int} bytes of whole characters")]
fn default_line(_: &mut Sdk, bytes: usize) {
    use workshop_ext_sdk::log::{DEFAULT_LINE, Line, line};
    let long = "é".repeat(DEFAULT_LINE);
    let l: Line = line(format_args!("x{long}"));
    assert_eq!(l.len(), bytes - 1);
    assert!(l.as_str().starts_with("xé") && !l.is_empty());
    assert_eq!(l.to_string(), l.as_str());
    let full: Line = line(format_args!("{long}"));
    assert_eq!((full.as_bytes().len(), DEFAULT_LINE), (bytes, bytes));
    assert!(Line::<4>::default().is_empty());
}

#[then(expr = "a 3-byte line keeps {string} of {string} and {string}")]
fn small_line(_: &mut Sdk, kept: String, a: String, b: String) {
    use core::fmt::Write as _;
    let mut l = workshop_ext_sdk::log::Line::<3>::new();
    l.write_str(&a).unwrap();
    l.write_str(&b).unwrap();
    assert_eq!(l.as_str(), kept);
}

// ---- sites ---------------------------------------------------------------

const HEAD: [u8; 9] = [0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x60, 0x89, 0x4D, 0xA0];
const IMAGE: Image = Image {
    base: 0x40_0000,
    size: 0x10_0000,
};
const ENTRY: usize = 0x43_e6e0;
const DEST: usize = 0x7000_0000;

workshop_ext_sdk::sites! {
    /// Every byte form and kind.
    const TABLE = {
        "call head" @ 0x3e6e0 = [0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x60, 0x89, 0x4D, 0xA0], call;
        "hook head" @ 0x3e6e0 = HEAD, hook;
        "anchor" @ (0x100 + 0x10) = b"\x01\x02", anchor;
        "slice anchor" @ 0x200 = { HEAD.split_at(3).0 } anchor;
    }
}

fn rel32(entry: usize, dest: usize) -> [u8; 5] {
    let b = (dest as u32).wrapping_sub(entry as u32 + 5).to_le_bytes();
    [0xE9, b[0], b[1], b[2], b[3]]
}

fn memory() -> FakeMemory {
    FakeMemory {
        executable: vec![(DEST, 0x1000)],
        dwords: vec![(0x6000_0000, DEST as u32)],
    }
}

fn reader(image: &[u8]) -> impl Fn(u32, &mut [u8]) -> bool + '_ {
    move |rva, out| match image.get(rva as usize..rva as usize + out.len()) {
        Some(src) => {
            out.copy_from_slice(src);
            true
        }
        None => false,
    }
}

impl Sdk {
    fn verify(&mut self, image: Image, memory: &dyn sites::Memory) {
        let mut findings = Vec::new();
        self.summary = sites::verify(TABLE, image, &reader(&self.image), memory, &mut |s, f| {
            findings.push((s.name, f))
        });
        self.findings = findings;
    }
}

#[given("the retail image with a call head, a hook head, an anchor and a slice anchor")]
fn retail_image(w: &mut Sdk) {
    w.image = vec![0xCC; 0x4_0000];
    for s in TABLE {
        w.image[s.rva as usize..s.rva as usize + s.bytes.len()].copy_from_slice(s.bytes);
    }
    assert_eq!(
        (TABLE.len(), TABLE[2].rva, TABLE[3].bytes),
        (4, 0x110, &HEAD[..3])
    );
}

#[then("verification passes with no findings")]
fn passes(w: &mut Sdk) {
    w.verify(IMAGE, &memory());
    assert!(w.summary.passed() && w.findings.is_empty());
}

#[when(
    expr = "another component writes a {string} detour at the head to executable memory outside the image"
)]
fn detour(w: &mut Sdk, jump: String) {
    assert_eq!(jump, Jump::Rel32.text());
    w.image[0x3e6e0..0x3e6e5].copy_from_slice(&rel32(ENTRY, DEST));
}

#[when("the anchor byte changes")]
fn anchor_changes(w: &mut Sdk) {
    w.image[0x110] ^= 0xff;
}

#[then(expr = "{int} sites fail and {int} carry a foreign detour")]
fn failed_foreign(w: &mut Sdk, failed: usize, foreign: usize) {
    w.verify(IMAGE, &memory());
    assert_eq!(w.summary, Summary { failed, foreign });
    assert!(!w.summary.passed());
}

#[then(expr = "the findings are {string}, {string} and {string}")]
fn findings(w: &mut Sdk, a: String, b: String, c: String) {
    let text: Vec<String> = w
        .findings
        .iter()
        .map(|(name, f)| {
            let what = match f {
                Finding::Foreign { .. } => "foreign",
                Finding::Problem(Problem::Occupied { .. }) => "occupied",
                Finding::Problem(Problem::Bytes) => "bytes",
                Finding::Problem(Problem::OutsideImage) => "outside",
            };
            format!("{name}: {what}")
        })
        .collect();
    assert_eq!(text, [a, b, c]);
}

#[then(expr = "classifying a head carrying {} gives {string}")]
fn classify(_: &mut Sdk, detour: String, verdict: String) {
    let with = |jump: &[u8]| {
        let mut found = HEAD.to_vec();
        found[..jump.len()].copy_from_slice(jump);
        found
    };
    let m = memory();
    let mut expected: &[u8] = &HEAD;
    let mut mem: &dyn sites::Memory = &m;
    let found = match detour.as_str() {
        "the retail bytes" => HEAD.to_vec(),
        "a rel32 jump to executable memory" => with(&rel32(ENTRY, DEST)),
        "an indirect jump through a readable slot" => with(&[0xFF, 0x25, 0, 0, 0, 0x60]),
        "an indirect jump through an unreadable slot" => with(&[0xFF, 0x25, 4, 0, 0, 0x60]),
        "a rel32 jump into the image" => with(&rel32(ENTRY, 0x40_1000)),
        "a rel32 jump to non-executable memory" => with(&rel32(ENTRY, 0x7100_0000)),
        "a rel32 jump to address zero" => with(&rel32(ENTRY, 0)),
        "a rel32 jump with a changed byte after it" => {
            let mut f = with(&rel32(ENTRY, DEST));
            f[8] ^= 1;
            f
        }
        "a shorter byte run" => HEAD[..8].to_vec(),
        "other bytes" => vec![0x90; 9],
        "a rel32 jump over a five-byte head" => {
            expected = &HEAD[..5];
            rel32(ENTRY, DEST).to_vec()
        }
        "a rel32 jump with no foreign code mapped" => {
            mem = &NoForeignCode;
            with(&rel32(ENTRY, DEST))
        }
        other => panic!("unknown detour {other}"),
    };
    let got = match sites::classify(ENTRY, expected, &found, IMAGE, mem) {
        Verdict::Retail => "retail",
        Verdict::Foreign { dest, .. } => {
            assert_eq!(dest, DEST);
            "foreign"
        }
        Verdict::Mismatch => "mismatch",
    };
    assert_eq!(got, verdict);
    assert_eq!(NoForeignCode.read_u32(0), None);
}

use sites::Memory as _;

#[then("call sites admit retail and foreign bytes, hook sites and anchors only retail")]
fn admits(_: &mut Sdk) {
    let foreign = Verdict::Foreign {
        jump: Jump::Indirect,
        dest: DEST,
    };
    for (kind, want) in [
        (SiteKind::Call, true),
        (SiteKind::Hook, false),
        (SiteKind::Anchor, false),
    ] {
        assert!(sites::admits(kind, Verdict::Retail));
        assert_eq!(sites::admits(kind, foreign), want);
        assert!(!sites::admits(kind, Verdict::Mismatch));
    }
    assert_eq!((Jump::Rel32.size(), Jump::Indirect.size()), (5, 6));
    assert!(!IMAGE.contains(u32::MAX, usize::MAX));
}

#[then(expr = "an image of 0x150 bytes fails all {int} sites, some outside the image")]
fn short_image(w: &mut Sdk, count: usize) {
    w.image[0x110] ^= 0xff;
    w.verify(
        Image {
            base: IMAGE.base,
            size: 0x150,
        },
        &NoForeignCode,
    );
    assert_eq!(w.summary.failed, count);
    assert!(
        w.findings
            .iter()
            .any(|f| f.1 == Finding::Problem(Problem::OutsideImage))
    );
    w.image[0x110] ^= 0xff;
}

#[then("an unreadable image fails every site")]
fn unreadable(_: &mut Sdk) {
    let summary = sites::verify(TABLE, IMAGE, &|_, _| false, &NoForeignCode, &mut |_, _| {});
    assert_eq!(summary.failed, TABLE.len());
}

const LONG: Site = Site {
    name: "too long",
    rva: 0,
    bytes: &[0; sites::MAX_SITE_BYTES + 1],
    kind: SiteKind::Anchor,
};

#[then("a site longer than 512 bytes is outside the comparable range")]
fn long_site(w: &mut Sdk) {
    assert_eq!(
        sites::check(&LONG, IMAGE, &reader(&w.image), &NoForeignCode),
        Err(Problem::OutsideImage)
    );
}

#[then("empty or oversized sites fail the table check")]
fn table_check(_: &mut Sdk) {
    assert!(!sites::check_table(&[LONG]));
    assert!(!sites::check_table(&[Site { bytes: &[], ..LONG }]));
    assert!(sites::check_table(TABLE));
}

#[then(expr = "the problems read {string}, {string} and {string}")]
fn problem_text(_: &mut Sdk, a: String, b: String, c: String) {
    let occupied = Problem::Occupied {
        jump: Jump::Indirect,
        dest: 0x1234,
    };
    assert_eq!(
        [
            Problem::OutsideImage.to_string(),
            Problem::Bytes.to_string(),
            occupied.to_string()
        ],
        [a, b, c]
    );
}

#[then(expr = "bytes display as {string}")]
fn bytes_text(_: &mut Sdk, text: String) {
    assert_eq!(sites::hex_bytes(&[0x0f, 0xb6, 0x45]).to_string(), text);
    assert_eq!(sites::hex_bytes(&[]).to_string(), "");
}

// ---- hooks ---------------------------------------------------------------

static SLOTS: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];

fn specs() -> [HookSpec<'static>; 3] {
    [0, 1, 2].map(|i| HookSpec {
        target: 0x40_0000 + 0x1000 * (i + 1),
        detour: 0x10 * (i + 1),
        original: &SLOTS[i],
    })
}

#[given(regex = r"^the host fails the (\d)\w\w hook creation$")]
fn fail_create(_: &mut Sdk, n: usize) {
    FakeHost::fail_hook_create_at(n - 1);
}

#[given(regex = r"^the host fails enabling the (\d)\w\w hook$")]
fn fail_enable(_: &mut Sdk, n: usize) {
    FakeHost::fail_hook_enable_at(n - 1);
}

#[given("the host hands back null trampolines")]
fn null_trampolines(_: &mut Sdk) {
    FakeHost::null_trampolines();
}

#[when(expr = "the unit installs {int} hooks")]
fn install(w: &mut Sdk, count: usize) {
    SLOTS.iter().for_each(|s| s.store(0, Ordering::Release));
    let mut failure = None;
    let rc = unsafe {
        unit::run_init(w.context(), |init| {
            init.install_hooks(&specs()[..count]).map_err(|f| {
                failure = Some(f);
                InitError::from(f)
            })
        })
    };
    assert_eq!(
        rc,
        if failure.is_some() {
            code::HOOK_FAILED
        } else {
            code::OK
        }
    );
    w.failure = failure;
}

#[then(expr = "{int} hooks are enabled and each detour holds its trampoline")]
fn enabled(_: &mut Sdk, count: usize) {
    let hooks = FakeHost::hooks();
    assert_eq!(hooks.len(), count);
    for (h, slot) in hooks.iter().zip(&SLOTS) {
        assert!(h.enabled);
        assert_eq!(slot.load(Ordering::Acquire), h.original);
    }
}

#[then("disabling, enabling and removing a hook reach the host")]
fn control(w: &mut Sdk) {
    let mut from_init = None;
    unsafe {
        unit::run_init(w.context(), |init| {
            from_init = init.hooks().map(|f| f.control());
            Ok(())
        })
    };
    assert!(from_init.is_some());
    let control = w.live().hooks().unwrap();
    assert_eq!(control.disable(0x40_1000), Ok(()));
    assert!(!FakeHost::hooks()[0].enabled);
    assert_eq!(control.enable(0x40_1000), Ok(()));
    assert_eq!(control.remove(0x40_1000), Ok(()));
    assert_eq!(control.remove(0x40_1000), Err(status::NOT_FOUND));
    assert_eq!(control.enable(0x40_1000), Err(status::NOT_FOUND));
    assert_eq!(control.disable(0x40_1000), Err(status::NOT_FOUND));
}

#[then(expr = "the install fails with {string}")]
fn install_fails(w: &mut Sdk, text: String) {
    assert_eq!(w.failure.expect("a failed install").to_string(), text);
}

#[then("no hook remains and every trampoline is cleared")]
fn rolled_back(_: &mut Sdk) {
    assert!(FakeHost::hooks().is_empty());
    assert!(SLOTS.iter().all(|s| s.load(Ordering::Acquire) == 0));
    let (disabled, removed) = FakeHost::hook_cleanups();
    assert_eq!(disabled, removed);
}

#[then("hooks can be controlled but not created")]
fn control_only(w: &mut Sdk) {
    assert!(w.live().hooks().is_some());
    let rc = unsafe {
        unit::run_init(w.context(), |init| {
            assert!(init.hooks().is_none());
            let f = init.install_hooks(&specs()).unwrap_err();
            assert_eq!(
                (f.stage, f.status, f.target),
                (HookStage::Create, status::UNAVAILABLE, 0x40_1000)
            );
            assert_eq!(f.stage.text(), "create");
            assert_eq!(init.install_hooks(&[]).unwrap_err().target, 0);
            Err(f.into())
        })
    };
    assert_eq!(rc, code::HOOK_FAILED);
    w.fake().context_mut().hook_remove = None;
    assert!(w.live().hooks().is_none());
}

// ---- storage -------------------------------------------------------------

#[when(expr = "the unit registers the region {string} in init")]
fn register_region(w: &mut Sdk, id: String) {
    let mut region = None;
    let rc = unsafe {
        unit::run_init(w.context(), |init| {
            region = init.storage(&id).unwrap();
            Ok(())
        })
    };
    assert_eq!(rc, 0);
    w.region = region;
}

impl Sdk {
    fn region(&self) -> Region {
        self.region.expect("a registered region")
    }
}

#[then("the region reads as never written")]
fn never_written(w: &mut Sdk) {
    assert_eq!(w.region().info(), Ok(None));
    assert_eq!(w.region().read_all(), Ok(None));
    assert_eq!(w.region().handle(), 1);
}

#[when(expr = "the unit writes {string} with schema {int}")]
fn write_region(w: &mut Sdk, text: String, schema: u32) {
    let revision = w.region().info().unwrap().map_or(0, |i| i.revision);
    w.region().write(revision, schema, text.as_bytes()).unwrap();
}

#[then(expr = "the region holds {string} with schema {int} at revision {int}")]
fn holds(w: &mut Sdk, text: String, schema: u32, revision: u64) {
    let info = RegionInfo {
        schema,
        length: text.len() as u64,
        revision,
    };
    assert_eq!(w.region().read_all(), Ok(Some((info, text.into_bytes()))));
    assert_eq!(FakeHost::regions()[0].revision, revision);
}

#[then(expr = "bytes 3..5 read {string} and bytes 4..6 are refused")]
fn ranged_read(w: &mut Sdk, text: String) {
    let mut two = [0u8; 2];
    assert_eq!(w.region().read(3, &mut two), Ok(()));
    assert_eq!(two, text.as_bytes());
    assert_eq!(w.region().read(4, &mut two), Err(status::INVALID_ARGUMENT));
}

#[then("a write against revision 0 is refused as a conflict")]
fn stale(w: &mut Sdk) {
    assert_eq!(w.region().write(0, 2, b"x"), Err(status::STATE_CONFLICT));
}

#[when(expr = "the unit writes {string} at 0 and {string} at 5 in a 6-byte batch")]
fn batch(w: &mut Sdk, a: String, b: String) {
    w.region()
        .write_ranges(1, 6, 3, &[(0, a.as_bytes()), (5, b.as_bytes())])
        .unwrap();
}

#[when("the unit empties the region")]
fn empty_region(w: &mut Sdk) {
    w.region().write(2, 3, b"").unwrap();
}

#[then("more than 256 ranges are refused as over the limit")]
fn too_many_ranges(w: &mut Sdk) {
    let ranges = vec![(0u64, &b""[..]); abi::STATE_MAX_RANGES + 1];
    assert_eq!(
        w.region().write_ranges(3, 0, 3, &ranges),
        Err(status::STATE_LIMIT)
    );
}

#[then("schema 0 is refused")]
fn schema_zero(w: &mut Sdk) {
    assert_eq!(w.region().write(3, 0, b"a"), Err(status::INVALID_ARGUMENT));
}

#[then("a flush reaches the host")]
fn flushes(w: &mut Sdk) {
    assert_eq!(w.region().flush(), Ok(()));
    assert_eq!(FakeHost::flushes(), 1);
    assert_eq!(
        RegionInfo::from(StateInfo::default()),
        RegionInfo::default()
    );
}

#[then(expr = "the region ID {string} is {word}")]
fn region_id(_: &mut Sdk, id: String, validity: String) {
    assert_eq!(storage::valid_id(&id), validity == "valid");
}

fn init_storage(w: &Sdk, id: &str) -> Result<Option<Region>, StorageError> {
    let mut out = None;
    unsafe {
        unit::run_init(w.context(), |init| {
            out = Some(init.storage(id));
            Ok(())
        })
    };
    out.unwrap()
}

#[then(expr = "registering {string} again is refused as a conflict")]
fn conflict(w: &mut Sdk, id: String) {
    let err = init_storage(w, &id).unwrap_err();
    assert_eq!(err, StorageError::Register(status::STATE_CONFLICT));
    assert_eq!(err.to_string(), "storage region registration failed (-8)");
    assert_eq!(
        init_storage(w, "bad id").unwrap_err(),
        StorageError::Register(status::INVALID_ARGUMENT)
    );
}

#[then("a host in recovery refuses registration and an IO error fails the write")]
fn recovery(w: &mut Sdk) {
    FakeHost::fail_services(
        status::STATE_RECOVERY_REQUIRED,
        status::STATE_IO_ERROR,
        0,
        0,
    );
    assert_eq!(
        init_storage(w, "other").unwrap_err(),
        StorageError::Register(status::STATE_RECOVERY_REQUIRED)
    );
    assert_eq!(w.region().write(0, 1, b"a"), Err(status::STATE_IO_ERROR));
    FakeHost::fail_services(0, 0, 0, 0);
    w.region().write(0, 1, b"a").unwrap();
}

#[then("a short info answer is a decode failure")]
fn short_info(w: &mut Sdk) {
    FakeHost::info_size(4);
    assert_eq!(w.region().info(), Err(status::DECODE_FAILED));
}

unsafe extern "C" fn zero_handle(_: *const u8, _: usize, h: *mut u32) -> i32 {
    unsafe { h.write(0) };
    0
}
unsafe extern "C" fn failing_flush() -> i32 {
    status::STATE_IO_ERROR
}
unsafe extern "C" fn huge(_: u32, out: *mut StateInfo) -> i32 {
    unsafe {
        out.write(StateInfo {
            size: size_of::<StateInfo>() as u32,
            schema: 1,
            length: abi::STATE_MAX_REGION_BYTES + 1,
            revision: 1,
            compression: 0,
            reserved: 0,
        })
    };
    0
}
static EMPTY_STATE: StateApi = StateApi {
    register_region: None,
    info: None,
    read_at: None,
    write_batch: None,
    flush: None,
    ..testing::STATE_API
};
static ODD_STATE: StateApi = StateApi {
    register_region: Some(zero_handle),
    flush: Some(failing_flush),
    info: Some(huge),
    ..testing::STATE_API
};

fn region_on(w: &mut Sdk, api: &'static StateApi) -> Result<Option<Region>, StorageError> {
    w.fake().context_mut().storage = Some(api);
    init_storage(w, "x")
}

#[then("a handle the host never issued is refused")]
fn ghost(w: &mut Sdk) {
    let id = "ghost";
    let mut region = init_storage(w, id).unwrap().unwrap();
    FakeHost::take_region("example.state");
    FakeHost::take_region(id);
    let _ = &mut region;
    assert_eq!(region.info(), Err(status::INVALID_ARGUMENT));
    assert_eq!(region.write(0, 1, b"a"), Err(status::INVALID_ARGUMENT));
}

#[then("a table without entries refuses every call but flush")]
fn empty_table(w: &mut Sdk) {
    assert_eq!(
        region_on(w, &EMPTY_STATE).unwrap_err(),
        StorageError::Service(ServiceError::StorageIncomplete)
    );
}

static FLUSHY_STATE: StateApi = StateApi {
    flush: Some(failing_flush),
    info: Some(huge),
    ..testing::STATE_API
};
static NO_FLUSH_STATE: StateApi = StateApi {
    flush: None,
    ..testing::STATE_API
};

#[then("a zero handle, a failing flush and an oversized region are reported")]
fn odd_table(w: &mut Sdk) {
    assert_eq!(
        region_on(w, &ODD_STATE).unwrap_err(),
        StorageError::Register(status::INVALID_ARGUMENT)
    );
    let odd = region_on(w, &FLUSHY_STATE).unwrap().unwrap();
    assert_eq!(odd.flush(), Err(status::STATE_IO_ERROR));
    assert_eq!(odd.read_all(), Err(status::STATE_LIMIT));
    FakeHost::take_region("x");
    let plain = region_on(w, &NO_FLUSH_STATE).unwrap().unwrap();
    assert_eq!(plain.flush(), Ok(()));
}

#[then("init sees no storage, settings or save events service")]
fn none_in_init(w: &mut Sdk) {
    let rc = unsafe {
        unit::run_init(w.context(), |init| {
            assert!(init.storage("x").unwrap().is_none());
            assert!(init.settings().unwrap().is_none());
            assert!(init.save_events().unwrap().is_none());
            Ok(())
        })
    };
    assert_eq!(rc, 0);
}

#[then("raw storage, settings and save event tables register through copies")]
fn raw_tables(_: &mut Sdk) {
    let state = testing::STATE_API;
    let region = unsafe { Region::register_raw(&state, "example.raw") }.unwrap();
    assert_eq!(region.write(0, 1, b"ok").unwrap().revision, 1);
    let settings_table = testing::SETTINGS_API;
    let settings = unsafe { settings::Settings::from_raw(&settings_table) };
    assert_eq!(settings.register("example.raw", declared::query), Ok(()));
    let saves_table = testing::SAVE_EVENTS_API;
    let saves = unsafe { save_events::SaveEvents::from_raw(&saves_table) };
    assert_eq!(saves.register("example.raw", 1, record), Ok(()));
    assert_eq!(FakeHost::save_callbacks().len(), 1);
}

// ---- settings ------------------------------------------------------------

static FLAG: AtomicBool = AtomicBool::new(false);
static LEVEL: AtomicI32 = AtomicI32::new(5);
static MODE: AtomicU32 = AtomicU32::new(0);
static SCALE: Mutex<f32> = Mutex::new(1.0);
static PRESSED: AtomicU32 = AtomicU32::new(0);

fn meta() -> Meta<'static> {
    Meta {
        id: "example.menu",
        name: "Example",
        author: "Tests",
        release: "1.0.0",
        help: "Help.",
        website: Some("https://example.invalid"),
    }
}

#[given("the example provider with every control kind")]
fn example_provider(w: &mut Sdk) {
    w.provider = Some(
        Builder::default()
            .status("status", "Status", || Cow::Borrowed("ready é"))
            .section("general", "General", "")
            .presentation(abi::SETTING_EXPERIMENTAL | settings::SETTING_COLLAPSED)
            .checkbox(
                "flag",
                "Flag",
                "",
                || FLAG.load(Ordering::Relaxed),
                |v| FLAG.store(v, Ordering::Relaxed),
            )
            .state(|| abi::SETTING_VISIBLE | abi::SETTING_ENABLED)
            .integer(
                "level",
                "Level",
                "",
                (1, 10, 1),
                || LEVEL.load(Ordering::Relaxed),
                |v| LEVEL.store(v, Ordering::Relaxed),
            )
            .display("Lv ", "", 0)
            .float(
                "scale",
                "Scale",
                "",
                (0.5, 2.0, 0.1),
                || *SCALE.lock().unwrap(),
                |v| *SCALE.lock().unwrap() = v,
            )
            .combo(
                "mode",
                "Mode",
                "",
                &["A", "B", "C"],
                || MODE.load(Ordering::Relaxed),
                |v| MODE.store(v, Ordering::Relaxed),
            )
            .button("press", "Press", "", || {
                PRESSED.fetch_add(1, Ordering::Relaxed);
            })
            .finish(&meta()),
    );
}

impl Sdk {
    fn controls(&self) -> &'static [SettingControl] {
        unsafe { settings::controls(self.provider.expect("a provider")) }
    }
}

fn ext<T: Copy>(c: &SettingControl) -> T {
    unsafe { *(c.extensions as *const T) }
}
fn read(c: &SettingControl) -> f32 {
    unsafe { c.read.unwrap()(c.user) }
}
fn write(c: &SettingControl, v: f32) {
    unsafe { c.write.unwrap()(c.user, v) }
}
fn text(p: *const core::ffi::c_char) -> &'static str {
    unsafe { settings::text(p) }
}

#[then(
    expr = "it has {int} controls of kinds status, section, checkbox, integer, float, combo, button"
)]
fn kinds(w: &mut Sdk, count: u32) {
    let p = w.provider.unwrap();
    assert_eq!(
        (p.count, p.version, p.size as usize),
        (count, 1, size_of::<SettingsProvider>())
    );
    let kinds: Vec<u32> = w.controls().iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            settings::SETTING_STATUS,
            abi::SETTING_SECTION,
            abi::SETTING_CHECKBOX,
            abi::SETTING_INTEGER,
            abi::SETTING_FLOAT,
            abi::SETTING_COMBO,
            abi::SETTING_BUTTON
        ]
    );
}

#[then(expr = "its metadata reads {string}, {string}, {string}, {string}, {string} and {string}")]
fn metadata(
    w: &mut Sdk,
    id: String,
    name: String,
    author: String,
    release: String,
    help: String,
    site: String,
) {
    let p = w.provider.unwrap();
    assert_eq!(
        [
            text(p.id),
            text(p.name),
            text(p.author),
            text(p.release),
            text(p.help),
            text(p.website)
        ],
        [id, name, author, release, help, site].map(|s| s.leak() as &str)
    );
    assert_eq!(text(core::ptr::null()), "");
    assert_eq!(text(w.controls()[2].id), "flag");
}

#[then(expr = "the status row reports {string} within {int} bytes, never splitting a character")]
fn status_row(w: &mut Sdk, shown: String, capacity: u32) {
    let t: SettingTextExtension = ext(&w.controls()[0]);
    assert_eq!((t.header.kind, t.refresh_ms), (abi::SETTING_EXT_TEXT, 250));
    let mut buf = vec![0xffu8; capacity as usize];
    let n = unsafe { t.read_text.unwrap()(t.user, buf.as_mut_ptr(), capacity) };
    assert_eq!(n, 8);
    assert_eq!(&buf[..shown.len() + 1], [shown.as_bytes(), b"\0"].concat());
    assert_eq!(
        unsafe { t.read_text.unwrap()(t.user, core::ptr::null_mut(), 0) },
        8
    );
}

#[then(
    expr = "the section is experimental and collapsed, the checkbox visible and the integer prefixed {string}"
)]
fn extensions(w: &mut Sdk, prefix: String) {
    let c = w.controls();
    let p: SettingPresentationExtension = ext(&c[1]);
    assert_eq!(
        p.flags,
        abi::SETTING_EXPERIMENTAL | settings::SETTING_COLLAPSED
    );
    let s: SettingStateExtension = ext(&c[2]);
    assert_eq!(
        unsafe { s.read_state.unwrap()(s.user) },
        abi::SETTING_VISIBLE | abi::SETTING_ENABLED
    );
    let d: SettingDisplayExtension = ext(&c[3]);
    assert_eq!(
        (text(d.prefix), text(d.suffix), d.decimals),
        (prefix.as_str(), "", 0)
    );
}

#[then(expr = "the combo offers {string}, {string} and {string}")]
fn options(w: &mut Sdk, a: String, b: String, c: String) {
    let combo = &w.controls()[5];
    let options =
        unsafe { core::slice::from_raw_parts(combo.options, combo.option_count as usize) };
    assert_eq!(
        options
            .iter()
            .map(|o| text(*o).to_string())
            .collect::<Vec<_>>(),
        [a, b, c]
    );
}

#[then("the checkbox turns on at 0.6 and off at 0.4, ignoring NaN")]
fn checkbox(w: &mut Sdk) {
    let c = &w.controls()[2];
    write(c, 0.6);
    assert_eq!(read(c), 1.0);
    write(c, 0.4);
    write(c, f32::NAN);
    assert!(!FLAG.load(Ordering::Relaxed));
}

#[then("the integer rounds 7.6 to 8, clamps 99 to 10 and ignores infinity")]
fn integer(w: &mut Sdk) {
    let c = &w.controls()[3];
    write(c, 7.6);
    assert_eq!(read(c), 8.0);
    write(c, 99.0);
    write(c, f32::INFINITY);
    assert_eq!(read(c), 10.0);
}

#[then("the float clamps 5 to 2 and ignores NaN")]
fn float(w: &mut Sdk) {
    let c = &w.controls()[4];
    write(c, 5.0);
    write(c, f32::NAN);
    assert_eq!(read(c), 2.0);
}

#[then("the combo rounds 1.4 to 1 and clamps -3 to 0")]
fn combo(w: &mut Sdk) {
    let c = &w.controls()[5];
    write(c, 1.4);
    assert_eq!(read(c), 1.0);
    write(c, -3.0);
    write(c, f32::NAN);
    assert_eq!(read(c), 0.0);
}

#[then("pressing the button runs it once")]
fn press(w: &mut Sdk) {
    let c = &w.controls()[6];
    let before = PRESSED.load(Ordering::Relaxed);
    unsafe { c.activate.unwrap()(c.user) };
    assert_eq!(PRESSED.load(Ordering::Relaxed), before + 1);
}

#[given("a provider whose callbacks all panic")]
fn panicking_provider(w: &mut Sdk) {
    w.provider = Some(
        Builder::new()
            .value(
                abi::SETTING_FLOAT,
                "v",
                "V",
                "",
                (0.0, 1.0, 0.0),
                || panic!("read"),
                |_| panic!("write"),
            )
            .button("b", "B", "", || panic!("press"))
            .state(|| panic!("state"))
            .status("s", "S", || panic!("text"))
            .finish(&Meta {
                website: None,
                ..meta()
            }),
    );
}

#[then("reads give 0, writes and presses are ignored and the status text is empty")]
fn panics_contained(w: &mut Sdk) {
    let c = w.controls();
    assert_eq!(read(&c[0]), 0.0);
    write(&c[0], 1.0);
    unsafe { c[1].activate.unwrap()(c[1].user) };
    let t: SettingTextExtension = ext(&c[2]);
    let mut buf = [1u8; 4];
    assert_eq!(
        unsafe { t.read_text.unwrap()(t.user, buf.as_mut_ptr(), 4) },
        0
    );
    assert_eq!(buf[0], 0);
    assert!(w.provider.unwrap().website.is_null());
    assert_eq!(settings::whole(2.5, 0, 9), Some(3));
}

#[then("a panicking state callback reads as visible")]
fn panicking_state(w: &mut Sdk) {
    let s: SettingStateExtension = ext(&w.controls()[1]);
    assert_eq!(
        unsafe { s.read_state.unwrap()(s.user) },
        abi::SETTING_VISIBLE
    );
}

#[then(expr = "building {} panics with {string}")]
fn builder_panics(_: &mut Sdk, mistake: String, message: String) {
    let build = move || {
        let b = Builder::new();
        let _ = match mistake.as_str() {
            "a duplicate id" => b.section("a", "", "").section("a", "", ""),
            "an empty id" => b.section("", "", ""),
            "a NUL in an id" => b.section("a\0", "", ""),
            "a 193-byte label" => b.section("a", &"x".repeat(193), ""),
            "an extension first" => b.state(|| 0),
            "two presentations" => b.section("a", "", "").presentation(1).presentation(2),
            "a combo without options" => b.combo("c", "", "", &[], || 0, |_| ()),
            "an integer step of 0" => b.integer("i", "", "", (0, 1, 0), || 0, |_| ()),
            "an inverted float range" => b.float("f", "", "", (2.0, 1.0, 0.0), || 0.0, |_| ()),
            "7 display decimals" => b.section("a", "", "").display("", "", 7),
            "257 controls" => {
                let mut b = b;
                for i in 0..abi::SETTINGS_MAX_CONTROLS {
                    b = b.section(&format!("s{i}"), "", "");
                }
                assert_eq!(b.len(), 256);
                assert!(!b.is_empty());
                b.section("more", "", "")
            }
            other => panic!("unknown mistake {other}"),
        };
    };
    let e = std::panic::catch_unwind(build).unwrap_err();
    let text = e.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(text.contains(&message), "{text}");
}

static ON: AtomicBool = AtomicBool::new(true);

workshop_ext_sdk::settings! {
    /// The declared provider.
    mod declared {
        id: "example.declared",
        name: "Declared",
        author: "Tests",
        release: "2",
        help: "",
        website: "https://example.invalid",
        game: GAME_XIII,
        controls: |b| b.checkbox("on", "On", "", || ON.load(Ordering::Relaxed), |v| ON.store(v, Ordering::Relaxed)),
    }
}

workshop_ext_sdk::settings! {
    mod any_game {
        id: "example.any",
        name: "Any",
        author: "",
        release: "",
        help: "",
        game: 0,
        controls: |b| b.section("only", "Only", ""),
    }
}

workshop_ext_sdk::settings! {
    mod broken {
        id: "example.broken",
        name: "Broken",
        author: "",
        release: "",
        help: "",
        game: 0,
        controls: |b| b.section("x", "", "").section("x", "", ""),
    }
}

fn query_context(game: u32) -> SettingsQueryContext {
    SettingsQueryContext {
        size: size_of::<SettingsQueryContext>() as u32,
        version: abi::SETTINGS_VERSION,
        game,
        game_name: core::ptr::null(),
        backend_version: core::ptr::null(),
    }
}

#[then("the declared provider answers XIII queries only, once built")]
fn declared_answers(_: &mut Sdk) {
    let p = declared::descriptor();
    assert!(core::ptr::eq(p, declared::descriptor()));
    assert_eq!(text(p.website), "https://example.invalid");
    assert_eq!(
        unsafe { declared::query(&query_context(GAME_XIII)) },
        p as *const _
    );
    assert!(unsafe { declared::query(&query_context(2)) }.is_null());
    assert!(unsafe { declared::query(core::ptr::null()) }.is_null());
}

#[then("a provider for any game answers every game")]
fn any_answers(_: &mut Sdk) {
    assert!(!unsafe { any_game::query(&query_context(3)) }.is_null());
}

#[then("a provider whose build panics answers null")]
fn broken_answers(_: &mut Sdk) {
    assert!(unsafe { broken::query(&query_context(1)) }.is_null());
}

#[then("query contexts of another version or size are refused")]
fn contexts(_: &mut Sdk) {
    let mut c = query_context(1);
    c.version = 2;
    assert!(!settings::accepts(Some(&c), 1));
    c = query_context(1);
    c.size = 4;
    assert!(!settings::accepts(Some(&c), 0));
    assert!(!settings::accepts(None, 0));
}

fn register_declared(w: &Sdk) -> Result<(), i32> {
    let mut out = Ok(());
    unsafe {
        unit::run_init(w.context(), |init| {
            out = init
                .settings()?
                .unwrap()
                .register(declared::ID, declared::query);
            Ok(())
        })
    };
    out
}

#[then(expr = "the host's menu sees {int} control")]
fn menu_sees(w: &mut Sdk, count: u32) {
    assert_eq!(register_declared(w), Ok(()));
    assert_eq!(FakeHost::query(declared::ID).map(|p| p.count), Some(count));
    assert!(FakeHost::query("nobody").is_none());
    assert_eq!(FakeHost::providers().len(), 1);
}

#[then("registering it again is refused as a conflict")]
fn provider_conflict(w: &mut Sdk) {
    assert_eq!(register_declared(w), Err(status::STATE_CONFLICT));
}

#[then("an unavailable registry is reported")]
fn registry_unavailable(w: &mut Sdk) {
    FakeHost::fail_services(0, 0, status::UNAVAILABLE, 0);
    assert_eq!(register_declared(w), Err(status::UNAVAILABLE));
    static NONE: SettingsApi = SettingsApi {
        register_provider: None,
        ..testing::SETTINGS_API
    };
    let _ = &NONE;
    let empty = SettingsProvider {
        controls: core::ptr::null(),
        count: 3,
        ..*declared::descriptor()
    };
    assert!(unsafe { settings::controls(&empty) }.is_empty());
}

// ---- save events ---------------------------------------------------------

static SEEN: Mutex<Vec<(u32, Option<u32>)>> = Mutex::new(Vec::new());

fn record(e: &SaveEvent) -> i32 {
    SEEN.lock().unwrap().push((e.kind, e.valid_slot()));
    0
}

fn boom(_: &SaveEvent) -> i32 {
    panic!("handler bug")
}

#[then("every event kind has its ABI value and mask")]
fn event_kinds(_: &mut Sdk) {
    for (i, k) in EventKind::ALL.iter().enumerate() {
        assert_eq!((k.raw(), k.mask()), (i as u32 + 1, 1 << i));
        assert_eq!(EventKind::from_raw(k.raw()), Some(*k));
    }
    assert_eq!(EventKind::from_raw(9), None);
    assert_eq!(save_events::mask_of(&EventKind::ALL), abi::SAVE_ALL_EVENTS);
}

#[then("a slot or profile is reported only when the host flags it valid")]
fn slot_flags(_: &mut Sdk) {
    let mut e = save_event(abi::LOADED, Some(3));
    assert_eq!(
        (e.event_kind(), e.valid_slot(), e.valid_profile()),
        (Some(EventKind::Loaded), Some(3), None)
    );
    e.flags |= abi::SAVE_PROFILE_VALID;
    assert_eq!(e.valid_profile(), Some(&[0; 32]));
    e.flags = 0;
    assert_eq!(e.valid_slot(), None);
    let mut no_slot = save_event(abi::SAVED, Some(1));
    no_slot.slot = abi::SAVE_NO_SLOT;
    assert_eq!(no_slot.valid_slot(), None);
}

fn with_saves<R>(w: &Sdk, f: impl FnOnce(&save_events::SaveEvents) -> R) -> R {
    let mut out = None;
    unsafe {
        unit::run_init(w.context(), |init| {
            out = Some(f(&init.save_events()?.unwrap()));
            Ok(())
        })
    };
    out.unwrap()
}

#[when("the unit subscribes to saved and new-game events and a panicking loaded handler")]
fn subscribe(w: &mut Sdk) {
    let mask = save_events::mask_of(&[EventKind::Saved, EventKind::NewGame]);
    with_saves(w, |s| {
        assert_eq!(s.register("example.saves", mask, record), Ok(()));
        assert_eq!(
            s.register("example.boom", EventKind::Loaded.mask(), boom),
            Ok(())
        );
    });
}

#[then(expr = "the adapter reports all events over {int} slots")]
fn adapter_info(w: &mut Sdk, slots: u32) {
    let info: SaveEventInfo = with_saves(w, |s| s.info().unwrap());
    assert_eq!(
        (info.events, info.slot_count),
        (abi::SAVE_ALL_EVENTS, slots)
    );
}

#[then("a saved event for slot 2 and a new game reach the handler, a save begin does not")]
fn delivered(_: &mut Sdk) {
    SEEN.lock().unwrap().clear();
    assert_eq!(FakeHost::fire(abi::SAVED, Some(2)), [0]);
    assert_eq!(FakeHost::fire(abi::NEW_GAME, None), [0]);
    assert!(FakeHost::fire(abi::SAVE_BEGIN, Some(2)).is_empty());
    assert_eq!(
        *SEEN.lock().unwrap(),
        [(abi::SAVED, Some(2)), (abi::NEW_GAME, None)]
    );
}

#[then(expr = "the panicking handler answers {int} instead of unwinding")]
fn panicking_handler(_: &mut Sdk, rc: i32) {
    assert_eq!(FakeHost::fire(abi::LOADED, Some(1)), [rc]);
    assert_eq!(rc, HANDLER_PANICKED);
}

#[then("null and short events are refused")]
fn bad_events(_: &mut Sdk) {
    let callbacks = FakeHost::save_callbacks();
    let (cb, user) = (callbacks[0].1.callback, callbacks[0].1.user);
    assert_eq!(
        unsafe { cb(core::ptr::null(), user) },
        status::INVALID_ARGUMENT
    );
    let mut short = save_event(abi::SAVED, Some(1));
    short.size = 8;
    assert_eq!(unsafe { cb(&short, user) }, status::INVALID_ARGUMENT);
}

#[then("an empty mask, an unknown event bit and an empty ID are refused")]
fn bad_registration(w: &mut Sdk) {
    with_saves(w, |s| {
        assert_eq!(s.register("x", 0, record), Err(status::INVALID_ARGUMENT));
        assert_eq!(s.register("x", 64, record), Err(status::INVALID_ARGUMENT));
        assert_eq!(s.register("", 1, record), Err(status::INVALID_ARGUMENT));
    });
}

#[then("a conflicting adapter hook is reported")]
fn adapter_conflict(w: &mut Sdk) {
    FakeHost::fail_services(0, 0, 0, status::HOOK_FAILED);
    assert_eq!(
        with_saves(w, |s| s.register("x", 1, record)),
        Err(status::HOOK_FAILED)
    );
}

unsafe extern "C" fn no_info(_: *mut SaveEventInfo) -> i32 {
    status::UNAVAILABLE
}
static EMPTY_SAVES: SaveEventsApi = SaveEventsApi {
    info: None,
    ..testing::SAVE_EVENTS_API
};
static FAILING_SAVES: SaveEventsApi = SaveEventsApi {
    info: Some(no_info),
    ..testing::SAVE_EVENTS_API
};

#[then("a table without entries is unavailable")]
fn saves_unavailable(w: &mut Sdk) {
    w.fake().context_mut().save_events = Some(&EMPTY_SAVES);
    assert_eq!(with_saves(w, |s| s.info()), Err(status::UNAVAILABLE));
    w.fake().context_mut().save_events = Some(&FAILING_SAVES);
    assert_eq!(with_saves(w, |s| s.info()), Err(status::UNAVAILABLE));
}

// ---- packed records ------------------------------------------------------

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// The warp unit's host-region schema 2 options.
#[derive(Clone, Debug, PartialEq, workshop_ext_sdk::PackedRecord)]
#[packed(version = 1, schema = 2, migrate = from_legacy, valid = origin_marker)]
struct Options {
    #[packed(default = 1, max = 11)]
    destination: u8,
    #[packed(max = 2)]
    debug_page: u8,
    #[packed(key = "DebugPointPage1", max = 200)]
    debug_point_1: u8,
    #[packed(key = "DebugPointPage2", max = 200)]
    debug_point_2: u8,
    #[packed(key = "DebugPointPage3", max = 200)]
    debug_point_3: u8,
    #[packed(default = 20217, min = 1, max = 29999)]
    battle_id: u16,
    verbose_log: bool,
    #[packed(key = "EnableZone107Probe")]
    probe: bool,
    #[packed(key = "SkipArkEntryEvent")]
    skip_ark_entry: bool,
    #[packed(max = 9)]
    ark_activation: u8,
    ark_boss_story_callback: bool,
    #[packed(max = u32::MAX)]
    return_zone: u32,
    #[packed(key = "Probe107OriginZone", max = u32::MAX)]
    probe_zone: u32,
    #[packed(key = "Probe107OriginValid")]
    probe_valid: bool,
    #[packed(text)]
    text: Vec<TextEntry>,
}

const LEGACY_LAYOUT: Layout = Layout {
    version: 1,
    schema: 1,
    fields: &[
        Field::new("Enabled", 1, 0, 1),
        Field::new("Destination", 1, 0, 11),
        Field::new("DebugPage", 0, 0, 2),
        Field::new("DebugPointPage1", 0, 0, 200),
        Field::new("DebugPointPage2", 0, 0, 200),
        Field::new("DebugPointPage3", 0, 0, 200),
        Field::new("BattleId", 20217, 1, 29999),
        Field::new("VerboseLog", 0, 0, 1),
        Field::new("EnableZone107Probe", 0, 0, 1),
        Field::new("SkipArkEntryEvent", 0, 0, 1),
        Field::new("ArkActivation", 0, 0, 9),
        Field::new("ArkBossStoryCallback", 0, 0, 1),
        Field::new("ReturnZone", 0, 0, 4_294_967_295),
        Field::new("Probe107OriginZone", 0, 0, 4_294_967_295),
        Field::new("Probe107OriginValid", 0, 0, 1),
    ],
};

fn from_legacy(schema: u32, bytes: &[u8]) -> Option<Options> {
    if schema != 1 {
        return None;
    }
    let (values, mut text) = packed::decode(&LEGACY_LAYOUT, bytes)?;
    text.retain(|e| !(e.section == 0 && e.key.eq_ignore_ascii_case("Enabled")));
    Some(Options::from_parts(&values[1..], text))
}

fn origin_marker(o: &Options) -> bool {
    !o.probe_valid || o.probe_zone != 0
}

#[then("the derived options layout equals the warp unit's schema 2 fields")]
fn derived_layout(_: &mut Sdk) {
    assert_eq!(Options::LAYOUT.fields, &LEGACY_LAYOUT.fields[1..]);
    assert_eq!(
        (
            Options::LAYOUT.version,
            Options::LAYOUT.schema,
            Options::LAYOUT.packed_len()
        ),
        (1, 2, 16)
    );
    assert_eq!(Options::LAYOUT.field("ReturnZone"), Some(11));
    assert_eq!(packed::field(Options::LAYOUT.fields, "nope"), None);
}

#[then(expr = "the default options encode to {string}")]
fn defaults_encode(_: &mut Sdk, bytes: String) {
    assert_eq!(Options::defaults().encode().unwrap(), hex(&bytes));
    assert_eq!(Options::decode(&hex(&bytes)), Some(Options::defaults()));
}

#[then(expr = "the full options round-trip {string}")]
fn full_round_trip(_: &mut Sdk, bytes: String) {
    let full = Options {
        destination: 11,
        debug_page: 2,
        debug_point_2: 199,
        battle_id: 10060,
        verbose_log: true,
        ark_activation: 9,
        return_zone: u32::MAX,
        probe_zone: 107,
        probe_valid: true,
        text: vec![
            TextEntry::setting("ReturnMapSet", "z105_ark"),
            TextEntry::setting("ReturnPos", "1.5 -2 300"),
            TextEntry {
                section: 3,
                key: "Other".into(),
                value: "kept".into(),
            },
        ],
        ..Options::defaults()
    };
    assert_eq!(full.encode().unwrap(), hex(&bytes));
    let back = Options::load(2, &hex(&bytes)).unwrap();
    assert_eq!(back.setting_text("RETURNPOS"), Some("1.5 -2 300"));
    assert_eq!(back.setting_text("Other"), None);
    assert_eq!(back, full);
}

#[then(expr = "the schema 1 record {string} migrates to destination {int} and return zone {int}")]
fn migrates(_: &mut Sdk, bytes: String, destination: u8, zone: u32) {
    let o = Options::load(1, &hex(&bytes)).unwrap();
    assert_eq!((o.destination, o.return_zone), (destination, zone));
}

#[then("an unknown schema is refused")]
fn unknown_schema(_: &mut Sdk) {
    assert_eq!(
        Options::load(9, &hex("010a0000007c27803401000000000000")),
        None
    );
}

#[then(expr = "the options record {string} is refused for {}")]
fn record_refused(_: &mut Sdk, bytes: String, _defect: String) {
    assert_eq!(packed::decode(&Options::LAYOUT, &hex(&bytes)), None);
    assert_eq!(Options::decode(&hex(&bytes)), None);
}

#[then(
    "encoding refuses short values, out-of-range values, bad or duplicate text, oversized and overlong records"
)]
fn encode_refusals(_: &mut Sdk) {
    let layout = Options::LAYOUT;
    let values = layout.defaults();
    assert_eq!(packed::encode(&layout, &values[1..], &[]), None);
    let mut bad = values.clone();
    bad[0] = 12;
    assert_eq!(packed::encode(&layout, &bad, &[]), None);
    for entry in [
        TextEntry::setting("", "x"),
        TextEntry::setting("destination", "x"),
        TextEntry::setting("a\0", "x"),
        TextEntry::setting("a", "\0"),
        TextEntry::setting(&"k".repeat(256), "x"),
        TextEntry::setting("a", &"v".repeat(packed::MAX_VALUE + 1)),
        TextEntry {
            section: 5,
            ..TextEntry::setting("a", "")
        },
    ] {
        assert_eq!(packed::encode(&layout, &values, &[entry]), None);
    }
    let dup = [TextEntry::setting("a", ""), TextEntry::setting("A", "")];
    assert_eq!(packed::encode(&layout, &values, &dup), None);
    let big: Vec<TextEntry> = (0..17)
        .map(|i| TextEntry::setting(&format!("k{i}"), &"v".repeat(4000)))
        .collect();
    assert_eq!(packed::encode(&layout, &values, &big), None);
    let many: Vec<TextEntry> = (0..=packed::MAX_TEXT)
        .map(|i| TextEntry::setting(&format!("k{i}"), ""))
        .collect();
    assert_eq!(packed::encode(&layout, &values, &many), None);
}

#[then("a record failing its validity hook neither encodes nor decodes")]
fn validity(_: &mut Sdk) {
    let invalid = Options {
        probe_valid: true,
        probe_zone: 0,
        ..Options::defaults()
    };
    assert_eq!(invalid.encode(), None);
    let raw = packed::encode(&Options::LAYOUT, &invalid.values(), &[]).unwrap();
    assert_eq!(Options::decode(&raw), None);
}

#[then("an oversized record and too many text entries are refused on decode")]
fn decode_limits(_: &mut Sdk) {
    assert_eq!(
        packed::decode(&Options::LAYOUT, &vec![1u8; packed::LIMIT + 1]),
        None
    );
    let mut many = hex("0101000000be13000000000000000000");
    for i in 0..=packed::MAX_TEXT {
        let key = format!("k{i}");
        many.extend([1, key.len() as u8, 0, 0]);
        many.extend(key.as_bytes());
    }
    assert_eq!(packed::decode(&Options::LAYOUT, &many), None);
}

#[derive(Debug, PartialEq, workshop_ext_sdk::PackedRecord)]
struct Minimal {
    on: bool,
    #[packed(min = -3, max = 3)]
    delta: i8,
}

#[then("a minimal record packs to 2 bytes and back")]
fn minimal(_: &mut Sdk) {
    let m = Minimal {
        on: true,
        delta: -3,
    };
    let bytes = m.encode().unwrap();
    assert_eq!(bytes, [1, 1]);
    assert_eq!(
        Minimal::decode(&bytes),
        Some(Minimal {
            on: true,
            delta: -3
        })
    );
    assert!(m.text().is_empty());
    assert_eq!(Minimal::load(1, &bytes), Minimal::decode(&bytes));
    assert_eq!(Minimal::load(4, &bytes), None);
    assert_eq!(
        Minimal::from_parts(&[], Vec::new()),
        Minimal {
            on: false,
            delta: -3
        }
    );
}

#[then("invalid layouts are detected")]
fn invalid_layouts(_: &mut Sdk) {
    const DUPLICATE: Layout = Layout {
        fields: &[Field::new("a", 0, 0, 1), Field::new("A", 0, 0, 1)],
        ..Minimal::LAYOUT
    };
    const DEFAULT_OUT: Layout = Layout {
        fields: &[Field::new("a", 5, 0, 1)],
        ..Minimal::LAYOUT
    };
    const OVERFLOW: Layout = Layout {
        fields: &[Field::new("a", 0, i64::MIN, i64::MAX)],
        ..Minimal::LAYOUT
    };
    const INVERTED: Layout = Layout {
        fields: &[Field::new("a", 0, 1, 0)],
        ..Minimal::LAYOUT
    };
    for layout in [DUPLICATE, DEFAULT_OUT, OVERFLOW, INVERTED] {
        assert!(!layout.is_valid());
    }
    assert!(Minimal::LAYOUT.is_valid());
    assert_eq!(
        (
            Field::new("x", 0, 0, 0).bits(),
            Field::new("x", 0, 1, 29999).bits()
        ),
        (0, 15)
    );
}

#[then("every integer type converts")]
fn conversions(_: &mut Sdk) {
    assert_eq!(
        [
            u8::MAX.to_packed(),
            u16::MAX.to_packed(),
            u32::MAX.to_packed(),
            i8::MIN.to_packed(),
            i16::MIN.to_packed(),
            i32::MIN.to_packed(),
            5i64.to_packed(),
            true.to_packed()
        ],
        [
            255,
            65535,
            4_294_967_295,
            -128,
            -32768,
            -2_147_483_648,
            5,
            1
        ]
    );
    assert_eq!(
        (
            u8::from_packed(1),
            u16::from_packed(9),
            u32::from_packed(7),
            i8::from_packed(-1),
            i16::from_packed(-9),
            i32::from_packed(-1),
            i64::from_packed(4),
            bool::from_packed(0)
        ),
        (1, 9, 7, -1, -9, -1, 4, false)
    );
}

// ---- unit boundary -------------------------------------------------------

static STARTED: AtomicUsize = AtomicUsize::new(0);
static STOPPED: AtomicUsize = AtomicUsize::new(0);

fn start(init: &Init) -> InitResult {
    STARTED.fetch_add(1, Ordering::SeqCst);
    init.host()
        .info(format_args!("started on {}", init.host().game()));
    Ok(())
}

fn stop() {
    STOPPED.fetch_add(1, Ordering::SeqCst);
}

mod exports {
    use super::*;
    workshop_ext_sdk::ff13_unit! { init: start, shutdown: stop }
}

mod exports_without_shutdown {
    use super::*;
    workshop_ext_sdk::ff13_unit! { init: start }
}

#[then(expr = "init returns 0 and the unit logged {string}")]
fn init_ok(w: &mut Sdk, line: String) {
    assert_eq!(unsafe { exports::init(w.context()) }, code::OK);
    assert!(FakeHost::logged(&line));
}

#[then("init of a null context returns 1")]
fn init_null(_: &mut Sdk) {
    assert_eq!(
        unsafe { exports::init(core::ptr::null()) },
        code::UNUSABLE_CONTEXT
    );
}

#[then(regex = r"^init of an? (version 2|8-byte) context returns 1 without running the unit$")]
fn init_unusable(w: &mut Sdk, what: String) {
    let before = STARTED.load(Ordering::SeqCst);
    let c = w.fake().context_mut();
    if what == "version 2" {
        c.version = 2;
    } else {
        c.size = 8;
    }
    assert_eq!(
        unsafe { exports_without_shutdown::init(w.context()) },
        code::UNUSABLE_CONTEXT
    );
    assert_eq!(STARTED.load(Ordering::SeqCst), before);
    let c = w.fake().context_mut();
    c.version = 1;
    c.size = size_of::<HostContext>() as u32;
}

#[then("shutdown runs the unit's shutdown, with or without one, and contains panics")]
fn shutdowns(_: &mut Sdk) {
    let before = STOPPED.load(Ordering::SeqCst);
    exports::shutdown();
    exports_without_shutdown::shutdown();
    assert_eq!(STOPPED.load(Ordering::SeqCst), before + 1);
    unit::run_shutdown(|| panic!("shutdown bug"));
}

#[then(expr = "an init that {} returns {int}")]
fn init_outcome(w: &mut Sdk, outcome: String, expected: i32) {
    let f: fn(&Init) -> InitResult = match outcome.as_str() {
        "refuses" => |_| Err(InitError::REFUSED),
        "fails a hook" => |_| Err(InitError::HOOK_FAILED),
        "returns code 77" => |_| Err(InitError::code(77)),
        "returns code 0 as an error" => |_| Err(InitError::code(0)),
        "panics with a message" => |_| panic!("init bug {}", 7),
        "panics with a number" => |_| std::panic::panic_any(5u8),
        "panics with a static text" => |_| panic!("static"),
        other => panic!("unknown outcome {other}"),
    };
    assert_eq!(unsafe { unit::run_init(w.context(), f) }, expected);
    if expected == code::PANICKED {
        assert!(FakeHost::logged("unit panicked during init: "));
    }
}

#[then(expr = "init errors read {string} and convert from hook and service failures")]
fn init_errors(_: &mut Sdk, text: String) {
    assert_eq!(InitError::REFUSED.to_string(), text);
    assert_eq!(InitError::UNUSABLE_CONTEXT.value(), 1);
    let failure = HookFailure {
        stage: HookStage::Enable,
        target: 0,
        status: -6,
        removed: 0,
    };
    assert_eq!(InitError::from(failure), InitError::HOOK_FAILED);
    assert_eq!(
        InitError::from(ServiceError::StorageVersion),
        InitError::REFUSED
    );
}

static SLOT: AtomicUsize = AtomicUsize::new(0);

#[then("an init using storage, settings, save events and hooks succeeds")]
fn init_all(w: &mut Sdk) {
    let all = |init: &Init| {
        let region = init.storage("example.unit").unwrap().unwrap();
        region.write(0, 1, b"x").unwrap();
        init.settings()?.unwrap();
        init.save_events()?
            .unwrap()
            .register("example.unit", abi::SAVE_ALL_EVENTS, |_| 0)
            .map_err(InitError::code)?;
        init.install_hooks(&[HookSpec {
            target: 0x40_1000,
            detour: 0x10,
            original: &SLOT,
        }])?;
        Ok(())
    };
    assert_eq!(unsafe { unit::run_init(w.context(), all) }, 0);
    assert_eq!(
        (
            FakeHost::hooks().len(),
            FakeHost::regions()[0].bytes.as_slice()
        ),
        (1, &b"x"[..])
    );
}

#[then("a second registration of its region is a conflict")]
fn second_registration(w: &mut Sdk) {
    assert_eq!(
        init_storage(w, "example.unit").unwrap_err(),
        StorageError::Register(status::STATE_CONFLICT)
    );
}

#[then(expr = "a malformed storage table refuses init with code {int}")]
fn malformed_storage(w: &mut Sdk, expected: i32) {
    w.fake().context_mut().storage = Some(&BAD_STATE[1]);
    let bad = |init: &Init| {
        let err = init.storage("x").unwrap_err();
        assert_eq!(err.to_string(), "storage service version/size");
        Err(match err {
            StorageError::Service(e) => e.into(),
            StorageError::Register(rc) => InitError::code(rc),
        })
    };
    assert_eq!(unsafe { unit::run_init(w.context(), bad) }, expected);
}

#[then(expr = "without MinHook services the install fails at create and init returns {int}")]
fn without_minhook(w: &mut Sdk, expected: i32) {
    w.fake().context_mut().hook_create = None;
    let rc = unsafe { unit::run_init(w.context(), |init| Ok(init.install_hooks(&specs())?)) };
    assert_eq!(rc, expected);
}

#[then("a prefix-only context offers no optional service")]
fn prefix_only(w: &mut Sdk) {
    let prefix = abi::HostContextV1 {
        size: abi::prefix_size() as u32,
        ..unsafe { *w.context() }.prefix()
    };
    let rc = unsafe {
        unit::run_init(&prefix, |init| {
            assert!(init.storage("x").unwrap().is_none());
            assert!(init.settings().unwrap().is_none());
            assert!(init.save_events().unwrap().is_none());
            Ok(())
        })
    };
    assert_eq!(rc, 0);
    assert_eq!(HostContext::from_prefix(prefix).storage.map(|_| ()), None);
}

#[then("a PE32 image exporting init and shutdown is a unit")]
fn pe_unit(_: &mut Sdk) {
    let names =
        workshop_ext_sdk::pe::exports(&testing::pe_image(&["init", "shutdown"], 0x10b)).unwrap();
    assert!(workshop_ext_sdk::pe::is_unit(&names));
    assert_eq!(workshop_ext_sdk::pe::UNIT_EXPORTS, ["init", "shutdown"]);
}

#[then("images exporting more, nothing, another machine or a broken table are not")]
fn pe_not_unit(_: &mut Sdk) {
    use workshop_ext_sdk::pe::{exports, is_unit};
    assert!(!is_unit(
        &exports(&testing::pe_image(&["DllMain", "init", "shutdown"], 0x10b)).unwrap()
    ));
    assert_eq!(exports(&testing::pe_image(&[], 0x10b)), Some(vec![]));
    assert_eq!(exports(&testing::pe_image(&["init"], 0x20b)), None);
    assert_eq!(exports(b"MZ"), None);
    let mut broken = testing::pe_image(&["init"], 0x10b);
    broken[0x240..0x244].copy_from_slice(&0x9000u32.to_le_bytes());
    assert_eq!(exports(&broken), None);
}

fn main() {
    // Scenarios share the fake host's per-thread state: one at a time.
    futures::executor::block_on(async {
        let failed = Sdk::cucumber()
            .max_concurrent_scenarios(1)
            .fail_on_skipped()
            .run("tests/features")
            .await
            .execution_has_failed();
        if failed {
            std::process::exit(1);
        }
    });
}

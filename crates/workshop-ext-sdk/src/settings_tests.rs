use super::*;
use crate::testing::{self, FakeHost};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;

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

fn full() -> &'static SettingsProvider {
    Builder::default()
        .status("status", "Status", || Cow::Borrowed("ready é"))
        .section("general", "General", "")
        .presentation(abi::SETTING_EXPERIMENTAL | SETTING_COLLAPSED)
        .checkbox(
            "flag",
            "Flag",
            "A flag.",
            || FLAG.load(Ordering::Relaxed),
            |v| FLAG.store(v, Ordering::Relaxed),
        )
        .state(|| abi::SETTING_VISIBLE)
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
        .state(|| panic!("state bug"))
        .finish(&meta())
}

fn ext<T: Copy>(c: &SettingControl) -> T {
    unsafe { *(c.extensions as *const T) }
}

#[test]
fn a_built_provider_is_the_v1_wire_layout() {
    let p = full();
    assert_eq!(
        (p.size as usize, p.version, p.count),
        (size_of::<SettingsProvider>(), 1, 7)
    );
    assert_eq!(p.control_stride as usize, size_of::<SettingControl>());
    unsafe {
        assert_eq!(text(p.id), "example.menu");
        assert_eq!(
            (text(p.name), text(p.author), text(p.release)),
            ("Example", "Tests", "1.0.0")
        );
        assert_eq!(
            (text(p.help), text(p.website)),
            ("Help.", "https://example.invalid")
        );
        assert_eq!(text(core::ptr::null()), "");
    }
    let c = unsafe { controls(p) };
    let kinds: Vec<u32> = c.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            SETTING_STATUS,
            abi::SETTING_SECTION,
            abi::SETTING_CHECKBOX,
            abi::SETTING_INTEGER,
            abi::SETTING_FLOAT,
            abi::SETTING_COMBO,
            abi::SETTING_BUTTON,
        ]
    );
    assert_eq!(unsafe { text(c[2].id) }, "flag");
    // Status text extension.
    let t: SettingTextExtension = ext(&c[0]);
    assert_eq!((t.header.kind, t.refresh_ms), (abi::SETTING_EXT_TEXT, 250));
    let mut buf = [0xffu8; 8];
    let n = unsafe { t.read_text.unwrap()(t.user, buf.as_mut_ptr(), buf.len() as u32) };
    assert_eq!(n, 8);
    assert_eq!(&buf[..7], b"ready \0");
    assert_eq!(
        unsafe { t.read_text.unwrap()(t.user, core::ptr::null_mut(), 0) },
        8
    );
    // Section presentation, checkbox state, integer display.
    let pres: SettingPresentationExtension = ext(&c[1]);
    assert_eq!(pres.flags, abi::SETTING_EXPERIMENTAL | SETTING_COLLAPSED);
    assert!(pres.header.next.is_null());
    let state: SettingStateExtension = ext(&c[2]);
    assert_eq!(
        unsafe { state.read_state.unwrap()(state.user) },
        abi::SETTING_VISIBLE
    );
    let display: SettingDisplayExtension = ext(&c[3]);
    assert_eq!(
        (unsafe { text(display.prefix) }, display.decimals),
        ("Lv ", 0)
    );
    // A panicking state callback reads as visible.
    let state: SettingStateExtension = ext(&c[6]);
    assert_eq!(
        unsafe { state.read_state.unwrap()(state.user) },
        abi::SETTING_VISIBLE
    );
    let options = unsafe { core::slice::from_raw_parts(c[5].options, c[5].option_count as usize) };
    assert_eq!(
        options
            .iter()
            .map(|o| unsafe { text(*o) })
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
}

fn read(c: &SettingControl) -> f32 {
    unsafe { c.read.unwrap()(c.user) }
}
fn write(c: &SettingControl, v: f32) {
    unsafe { c.write.unwrap()(c.user, v) }
}

#[test]
fn callbacks_convert_round_clamp_and_contain_panics() {
    let p = full();
    let c = unsafe { controls(p) };
    write(&c[2], 0.6);
    assert!(FLAG.load(Ordering::Relaxed));
    assert_eq!(read(&c[2]), 1.0);
    write(&c[2], 0.4);
    write(&c[2], f32::NAN);
    assert!(!FLAG.load(Ordering::Relaxed));
    assert_eq!((c[3].min, c[3].max, c[3].step), (1.0, 10.0, 1.0));
    write(&c[3], 7.6);
    assert_eq!(LEVEL.load(Ordering::Relaxed), 8);
    write(&c[3], 99.0);
    assert_eq!(read(&c[3]), 10.0);
    write(&c[3], f32::INFINITY);
    assert_eq!(LEVEL.load(Ordering::Relaxed), 10);
    write(&c[4], 5.0);
    assert_eq!(read(&c[4]), 2.0);
    write(&c[4], f32::NAN);
    assert_eq!(read(&c[4]), 2.0);
    write(&c[5], 1.4);
    assert_eq!(MODE.load(Ordering::Relaxed), 1);
    write(&c[5], -3.0);
    assert_eq!(read(&c[5]), 0.0);
    write(&c[5], f32::NAN);
    unsafe { c[6].activate.unwrap()(c[6].user) };
    assert_eq!(PRESSED.load(Ordering::Relaxed), 1);
    // Panicking callbacks.
    let bad = Builder::new()
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
        .status("s", "S", || panic!("text"))
        .finish(&Meta {
            website: None,
            ..meta()
        });
    let c = unsafe { controls(bad) };
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
    assert!(bad.website.is_null());
    assert_eq!(whole(2.5, 0, 9), Some(3));
    assert_eq!(whole(f32::NEG_INFINITY, 0, 9), None);
}

fn panics(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let e = std::panic::catch_unwind(f).unwrap_err();
    e.downcast_ref::<String>().cloned().unwrap_or_default()
}

#[test]
fn malformed_descriptors_fail_loudly() {
    assert!(
        panics(|| drop(Builder::new().section("a", "", "").section("a", "", "")))
            .contains(error::DUPLICATE)
    );
    assert!(panics(|| drop(Builder::new().section("", "", ""))).contains(error::STRING));
    assert!(panics(|| drop(Builder::new().section("a\0", "", ""))).contains(error::STRING));
    assert!(
        panics(|| drop(Builder::new().section("a", &"x".repeat(193), ""))).contains(error::STRING)
    );
    assert!(panics(|| drop(Builder::new().state(|| 0))).contains(error::NO_CONTROL));
    assert!(
        panics(|| drop(
            Builder::new()
                .section("a", "", "")
                .presentation(1)
                .presentation(2)
        ))
        .contains(error::EXTENSION)
    );
    assert!(
        panics(|| drop(Builder::new().combo("c", "", "", &[], || 0, |_| ())))
            .contains(error::RANGE)
    );
    assert!(
        panics(|| drop(Builder::new().integer("i", "", "", (0, 1, 0), || 0, |_| ())))
            .contains(error::RANGE)
    );
    assert!(
        panics(|| drop(Builder::new().float("f", "", "", (2.0, 1.0, 0.0), || 0.0, |_| ())))
            .contains(error::RANGE)
    );
    assert!(
        panics(|| drop(Builder::new().section("a", "", "").display("", "", 7)))
            .contains(error::RANGE)
    );
    let mut b = Builder::new();
    assert!(b.is_empty());
    for i in 0..abi::SETTINGS_MAX_CONTROLS {
        b = b.section(&format!("s{i}"), "", "");
    }
    assert_eq!(b.len(), 256);
    assert!(panics(move || drop(b.section("one more", "", ""))).contains(error::TOO_MANY));
}

static ON: AtomicBool = AtomicBool::new(true);

crate::settings! {
    /// Macro under test.
    mod menu {
        id: "example.macro",
        name: "Macro",
        author: "Tests",
        release: "2",
        help: "",
        website: "https://example.invalid",
        game: abi::GAME_XIII,
        controls: |b| b.checkbox("on", "On", "", || ON.load(Ordering::Relaxed), |v| ON.store(v, Ordering::Relaxed)),
    }
}

crate::settings! {
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

crate::settings! {
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
        version: 1,
        game,
        game_name: core::ptr::null(),
        backend_version: core::ptr::null(),
    }
}

#[test]
fn the_macro_answers_matching_queries_and_registers() {
    assert_eq!(menu::ID, "example.macro");
    let p = menu::descriptor();
    assert!(core::ptr::eq(p, menu::descriptor()));
    assert_eq!(unsafe { text(p.website) }, "https://example.invalid");
    let xiii = query_context(abi::GAME_XIII);
    assert_eq!(unsafe { menu::query(&xiii) }, p as *const SettingsProvider);
    assert!(unsafe { menu::query(&query_context(2)) }.is_null());
    assert!(unsafe { menu::query(core::ptr::null()) }.is_null());
    let mut old = xiii;
    old.version = 2;
    assert!(!accepts(Some(&old), 1));
    old = xiii;
    old.size = 4;
    assert!(!accepts(Some(&old), 1));
    assert!(accepts(Some(&query_context(3)), 0));
    assert!(!unsafe { any_game::query(&query_context(3)) }.is_null());
    assert!(unsafe { broken::query(&query_context(1)) }.is_null());

    let _host = FakeHost::new(abi::GAME_XIII);
    let settings = Settings::new(&testing::SETTINGS_API);
    assert_eq!(settings.register(menu::ID, menu::query), Ok(()));
    assert_eq!(
        settings.register(menu::ID, menu::query),
        Err(abi::status::STATE_CONFLICT)
    );
    assert_eq!(FakeHost::query(menu::ID).map(|p| p.count), Some(1));
    assert!(FakeHost::query("nobody").is_none());
    FakeHost::fail_services(0, 0, abi::status::UNAVAILABLE, 0);
    assert_eq!(
        settings.register("x", menu::query),
        Err(abi::status::UNAVAILABLE)
    );
    static NONE: abi::SettingsApi = abi::SettingsApi {
        register_provider: None,
        ..testing::SETTINGS_API
    };
    assert_eq!(
        Settings::new(&NONE).register("x", menu::query),
        Err(abi::status::UNAVAILABLE)
    );
    let empty = SettingsProvider {
        controls: core::ptr::null(),
        count: 3,
        ..*p
    };
    assert!(unsafe { controls(&empty) }.is_empty());
}

#[test]
fn raw_registries_and_option_lists() {
    let _host = FakeHost::new(abi::GAME_XIII);
    let table = testing::SETTINGS_API;
    let settings = unsafe { Settings::from_raw(&table) };
    assert_eq!(settings.register("example.raw", menu::query), Ok(()));
    let p = Builder::new()
        .value(
            abi::SETTING_COMBO,
            "c",
            "C",
            "",
            (0.0, 1.0, 1.0),
            || 0.0,
            |_| (),
        )
        .options(&["x", "y"])
        .finish(&meta());
    let c = unsafe { controls(p) };
    assert_eq!(c[0].option_count, 2);
}

#[test]
fn param_controls_read_and_write_their_params() {
    static ON: Param = Param::flag("On", false);
    static HP: Param = Param::new("Hp", 150, 10, 2000);
    static MODE: Param = Param::new("Mode", 0, 0, 2);
    let p = Builder::new()
        .param_checkbox("on", "On", "", &ON)
        .param_integer("hp", "HP", "", 10, &HP)
        .param_combo("mode", "Mode", "", &["A", "B", "C"], &MODE)
        .finish(&meta());
    let c = unsafe { controls(p) };
    assert_eq!(
        c.iter()
            .map(|c| (c.kind, c.min, c.max, c.step))
            .collect::<Vec<_>>(),
        vec![
            (abi::SETTING_CHECKBOX, 0.0, 1.0, 1.0),
            (abi::SETTING_INTEGER, 10.0, 2000.0, 10.0),
            (abi::SETTING_COMBO, 0.0, 2.0, 1.0),
        ]
    );
    assert_eq!(c[2].option_count, 3);
    let (read, write) = (c[1].read.unwrap(), c[1].write.unwrap());
    unsafe {
        write(c[1].user, 777.4);
        assert_eq!(read(c[1].user), 777.0);
        write(c[1].user, f32::NAN);
        assert_eq!(HP.get(), 777, "NaN ignored");
        write(c[1].user, -5.0);
        assert_eq!(HP.get(), 10, "clamped");
        (c[0].write.unwrap())(c[0].user, 1.0);
        (c[2].write.unwrap())(c[2].user, 9.0);
    }
    assert!(ON.is_on());
    assert_eq!(MODE.get(), 2);
}

#[test]
#[should_panic(expected = "settings: invalid range or options")]
fn a_param_checkbox_needs_a_flag() {
    static WIDE: Param = Param::new("Wide", 1, 0, 5);
    let _ = Builder::new().param_checkbox("w", "W", "", &WIDE);
}

#[test]
#[should_panic(expected = "settings: invalid range or options")]
fn a_param_combo_needs_one_option_per_value() {
    static MODE: Param = Param::new("Mode", 0, 0, 2);
    let _ = Builder::new().param_combo("m", "M", "", &["A", "B"], &MODE);
}

#[test]
#[should_panic(expected = "settings: invalid range or options")]
fn a_param_slider_needs_a_step() {
    static HP: Param = Param::new("Hp", 1, 0, 5);
    let _ = Builder::new().param_integer("h", "H", "", 0, &HP);
}

#[test]
fn menu_strings_list_what_an_adapter_looks_up() {
    static LEVEL: Param = Param::new("Level", 1, 0, 9);
    static MODE: Param = Param::new("Mode", 0, 0, 1);
    let p = Builder::new()
        .status("st", "Status label", || Cow::Borrowed("x"))
        .section("s", "Section", "Section help.")
        .param_integer("l", "Level", "Level help.", 1, &LEVEL)
        .display("Lv ", "%", 0)
        .state(|| abi::SETTING_VISIBLE)
        .param_combo("m", "Mode", "", &["A", "Level"], &MODE)
        .finish(&meta());
    assert_eq!(
        unsafe { menu_strings(p) },
        [
            "Example",
            "Section",
            "Section help.",
            "Level",
            "Level help.",
            "Lv ",
            "%",
            "Mode",
            "A"
        ]
    );
}

//! Header drift: the C headers are parsed and every mirrored struct is
//! compared field by field (names, order, offsets for this target's pointer
//! size, x86 sizes) and every constant by value.
use super::*;
use crate::header::{self, Layout};
use core::mem::offset_of;

macro_rules! rust_layout {
    ($ty:ty; $($field:ident),+ $(,)?) => {
        (
            vec![$((stringify!($field), offset_of!($ty, $field))),+],
            size_of::<$ty>(),
        )
    };
}

fn host_layout(name: &str) -> Layout {
    header::all_structs()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("{name} not in the headers"))
        .layout(size_of::<usize>())
}

fn check(c_name: &str, rust: (Vec<(&str, usize)>, usize)) {
    let c = host_layout(c_name);
    let names: Vec<&str> = c.fields.iter().map(|f| f.0.as_str()).collect();
    let rust_names: Vec<&str> = rust.0.iter().map(|f| f.0).collect();
    assert_eq!(names, rust_names, "{c_name}: field names/order");
    let offsets: Vec<usize> = c.fields.iter().map(|f| f.1).collect();
    let rust_offsets: Vec<usize> = rust.0.iter().map(|f| f.1).collect();
    assert_eq!(offsets, rust_offsets, "{c_name}: offsets");
    assert_eq!(c.size, rust.1, "{c_name}: size");
}

#[test]
fn every_mirrored_struct_matches_the_c_headers() {
    check(
        "FF13HostContext",
        rust_layout!(HostContext; version, size, game, phase, game_base, game_size, log,
            vfs_size, vfs_read, hook_create, hook_enable, hook_disable, hook_remove,
            storage, settings, save_events),
    );
    check(
        "FF13StateInfo",
        rust_layout!(StateInfo; size, schema, length, revision, compression, reserved),
    );
    check(
        "FF13StateRange",
        rust_layout!(StateRange; offset, data, length),
    );
    check(
        "FF13StateApi",
        rust_layout!(StateApi; version, size, register_region, info, read_at, write_batch, flush),
    );
    check(
        "FF13SettingsQueryContext",
        rust_layout!(SettingsQueryContext; size, version, game, game_name, backend_version),
    );
    check(
        "FF13SettingExtension",
        rust_layout!(SettingExtension; size, kind, version, next),
    );
    check(
        "FF13SettingStateExtension",
        rust_layout!(SettingStateExtension; header, read_state, user),
    );
    check(
        "FF13SettingPresentationExtension",
        rust_layout!(SettingPresentationExtension; header, flags),
    );
    check(
        "FF13SettingDisplayExtension",
        rust_layout!(SettingDisplayExtension; header, prefix, suffix, decimals),
    );
    check(
        "FF13SettingControl",
        rust_layout!(SettingControl; size, kind, id, label, help, min, max, step, read, write,
            user, activate, option_count, options, extensions),
    );
    check(
        "FF13SettingsProvider",
        rust_layout!(SettingsProvider; size, version, id, name, count, controls,
            reserved_menu_count, reserved_menu_stride, reserved_menus, control_stride,
            author, release, help, website, reserved_color, reserved_flags),
    );
    check(
        "FF13SettingsApi",
        rust_layout!(SettingsApi; version, size, register_provider, visit_providers),
    );
    check(
        "FF13SaveEvent",
        rust_layout!(SaveEvent; version, size, game, variant, kind, slot, operation, flags,
            reserved, profile),
    );
    check(
        "FF13SaveEventInfo",
        rust_layout!(SaveEventInfo; size, game, variant, events, slot_count, reserved),
    );
    check(
        "FF13SaveEventsApi",
        rust_layout!(SaveEventsApi; version, size, info, register_callback),
    );
    // The prefix is the context through hook_remove.
    let prefix = rust_layout!(HostContextV1; version, size, game, phase, game_base, game_size,
        log, vfs_size, vfs_read, hook_create, hook_enable, hook_disable, hook_remove);
    let full = host_layout("FF13HostContext");
    assert_eq!(prefix.0.len(), 13);
    for (i, (name, offset)) in prefix.0.iter().enumerate() {
        assert_eq!(
            (*name, *offset),
            (full.fields[i].0.as_str(), full.fields[i].1)
        );
    }
    assert_eq!(prefix.1, prefix_size());
}

#[test]
fn x86_sizes_and_static_asserts_match_the_headers() {
    let x86 = |name: &str| {
        header::all_structs()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
            .layout(4)
    };
    let ctx = x86("FF13HostContext");
    assert_eq!(ctx.size as u32, CONTEXT_SIZE_X86);
    assert_eq!(ctx.fields[13].1 as u32, PREFIX_SIZE_X86);
    let asserts = header::static_asserts();
    assert!(!asserts.is_empty());
    for (field, value) in asserts {
        match field.as_str() {
            "sizeof" => assert_eq!(ctx.size, value),
            name => {
                let at = ctx.fields.iter().find(|f| f.0 == name).unwrap().1;
                assert_eq!(at, value, "offsetof {name}");
            }
        }
    }
    let offsets: Vec<usize> = ctx.fields.iter().map(|f| f.1).collect();
    assert_eq!(offsets, context_offsets(4));
    assert_eq!(&offsets[..13], prefix_offsets(4));
    assert_eq!(
        context_offsets(size_of::<usize>()),
        [
            offset_of!(HostContext, version),
            offset_of!(HostContext, size),
            offset_of!(HostContext, game),
            offset_of!(HostContext, phase),
            offset_of!(HostContext, game_base),
            offset_of!(HostContext, game_size),
            offset_of!(HostContext, log),
            offset_of!(HostContext, vfs_size),
            offset_of!(HostContext, vfs_read),
            offset_of!(HostContext, hook_create),
            offset_of!(HostContext, hook_enable),
            offset_of!(HostContext, hook_disable),
            offset_of!(HostContext, hook_remove),
            offset_of!(HostContext, storage),
            offset_of!(HostContext, settings),
            offset_of!(HostContext, save_events),
        ]
    );
    // x86 sizes the docs quote.
    assert_eq!(x86("FF13StateInfo").size, 32);
    assert_eq!(x86("FF13SaveEvent").size, 72);
    assert_eq!(x86("FF13SettingControl").size, 60);
    assert_eq!(x86("FF13SettingsProvider").size, 76);
}

#[test]
fn every_constant_matches_the_headers() {
    let defines = header::defines();
    let expect: &[(&str, i64)] = &[
        ("FF13_HOST_ABI_VERSION", HOST_ABI_VERSION.into()),
        ("FF13_PHASE_EARLY", PHASE_EARLY.into()),
        ("FF13_PHASE_LATE", PHASE_LATE.into()),
        ("FF13_OK", status::OK.into()),
        ("FF13_INVALID_ARGUMENT", status::INVALID_ARGUMENT.into()),
        ("FF13_NOT_FOUND", status::NOT_FOUND.into()),
        ("FF13_SHORT_BUFFER", status::SHORT_BUFFER.into()),
        ("FF13_DECODE_FAILED", status::DECODE_FAILED.into()),
        ("FF13_UNAVAILABLE", status::UNAVAILABLE.into()),
        ("FF13_HOOK_FAILED", status::HOOK_FAILED.into()),
        ("FF13_LOG_ERROR", level::ERROR.into()),
        ("FF13_LOG_WARN", level::WARN.into()),
        ("FF13_LOG_INFO", level::INFO.into()),
        ("FF13_LOG_DEBUG", level::DEBUG.into()),
        ("FF13_STATE_VERSION", STATE_VERSION.into()),
        ("FF13_STATE_IO_ERROR", status::STATE_IO_ERROR.into()),
        ("FF13_STATE_CONFLICT", status::STATE_CONFLICT.into()),
        ("FF13_STATE_LIMIT", status::STATE_LIMIT.into()),
        (
            "FF13_STATE_RECOVERY_REQUIRED",
            status::STATE_RECOVERY_REQUIRED.into(),
        ),
        ("FF13_SETTINGS_VERSION", SETTINGS_VERSION.into()),
        ("FF13_SETTING_CHECKBOX", SETTING_CHECKBOX.into()),
        ("FF13_SETTING_INTEGER", SETTING_INTEGER.into()),
        ("FF13_SETTING_FLOAT", SETTING_FLOAT.into()),
        ("FF13_SETTING_BUTTON", SETTING_BUTTON.into()),
        ("FF13_SETTING_COMBO", SETTING_COMBO.into()),
        ("FF13_SETTING_SECTION", SETTING_SECTION.into()),
        ("FF13_SETTING_EXT_STATE", SETTING_EXT_STATE.into()),
        (
            "FF13_SETTING_EXT_PRESENTATION",
            SETTING_EXT_PRESENTATION.into(),
        ),
        ("FF13_SETTING_EXT_DISPLAY", SETTING_EXT_DISPLAY.into()),
        ("FF13_SETTING_VISIBLE", SETTING_VISIBLE.into()),
        ("FF13_SETTING_ENABLED", SETTING_ENABLED.into()),
        ("FF13_SETTING_BUSY", SETTING_BUSY.into()),
        ("FF13_SETTING_ADVANCED", SETTING_ADVANCED.into()),
        (
            "FF13_SETTING_RESTART_REQUIRED",
            SETTING_RESTART_REQUIRED.into(),
        ),
        ("FF13_SETTING_EXPERIMENTAL", SETTING_EXPERIMENTAL.into()),
        ("FF13_SAVE_EVENTS_VERSION", SAVE_EVENTS_VERSION.into()),
        ("FF13_SAVE_BEGIN", SAVE_BEGIN.into()),
        ("FF13_SAVED", SAVED.into()),
        ("FF13_SAVE_FAILED", SAVE_FAILED.into()),
        ("FF13_LOADED", LOADED.into()),
        ("FF13_NEW_GAME", NEW_GAME.into()),
        ("FF13_SAVE_ALL_EVENTS", SAVE_ALL_EVENTS.into()),
        ("FF13_SAVE_SLOT_VALID", SAVE_SLOT_VALID.into()),
        ("FF13_SAVE_PROFILE_VALID", SAVE_PROFILE_VALID.into()),
        ("FF13_SAVE_NO_SLOT", SAVE_NO_SLOT.into()),
    ];
    for (name, value) in expect {
        assert_eq!(defines.get(*name), Some(value), "{name}");
    }
    // Every numeric define of the headers is mirrored (macros excluded).
    let mirrored: Vec<&str> = expect.iter().map(|e| e.0).collect();
    for name in defines.keys() {
        assert!(mirrored.contains(&name.as_str()), "{name} is not mirrored");
    }
    assert_eq!(
        save_event_mask(NEW_GAME) | save_event_mask(SAVE_BEGIN),
        0b1_0001
    );
    assert_eq!(
        (1..=5).map(save_event_mask).fold(0, |a, b| a | b),
        SAVE_ALL_EVENTS
    );
    // The header's comment bounds are the SDK's limits.
    let text = header::text();
    for bound in [
        "256 controls/provider",
        "64-byte IDs",
        "128-byte name",
        "192-byte labels",
        "1024-byte help",
        "at most 24 bytes each",
        "bounded\n * to 16 records",
        "256 regions, 4 MiB per region, 256 ranges / 64 KiB per batch or read",
        "IDs (1..64 ASCII letters/digits/._-)",
        "at most 4096 bytes are logged",
    ] {
        assert!(text.contains(bound), "{bound}");
    }
    assert_eq!(
        (
            SETTINGS_MAX_CONTROLS,
            SETTINGS_MAX_ID,
            SETTINGS_MAX_NAME,
            SETTINGS_MAX_LABEL,
            SETTINGS_MAX_HELP,
            SETTINGS_MAX_AFFIX,
            SETTINGS_MAX_EXTENSIONS
        ),
        (256, 64, 128, 192, 1024, 24, 16)
    );
    assert_eq!(
        (
            STATE_MAX_REGIONS,
            STATE_MAX_REGION_BYTES,
            STATE_MAX_RANGES,
            STATE_MAX_BATCH_BYTES,
            STATE_MAX_ID,
            LOG_MESSAGE_MAX
        ),
        (256, 4 << 20, 256, 64 << 10, 64, 4096)
    );
}

#[test]
fn the_text_extension_matches_the_host_rust_contract() {
    let contract = header::SETTINGS_CONTRACT;
    let body = &contract[contract.find("pub struct TextExtension").unwrap()..];
    let body = &body[..body.find('}').unwrap()];
    let names: Vec<&str> = body
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|l| l.split_once(':').map(|f| f.0))
        .collect();
    assert_eq!(names, ["header", "callback", "user", "refresh_ms"]);
    assert_eq!(
        offset_of!(SettingTextExtension, read_text),
        size_of::<SettingExtension>()
    );
    assert_eq!(
        offset_of!(SettingTextExtension, refresh_ms),
        size_of::<SettingExtension>() + 2 * size_of::<usize>()
    );
    assert_eq!(SETTING_EXT_TEXT, 0x1002);
}

#[test]
fn games_have_names() {
    assert_eq!(game_name(GAME_XIII), "FINAL FANTASY XIII");
    assert_eq!(game_name(GAME_XIII_2), "FINAL FANTASY XIII-2");
    assert_eq!(game_name(GAME_LR), "LIGHTNING RETURNS: FINAL FANTASY XIII");
    assert_eq!(game_name(9), "unknown game");
}

unsafe extern "C" fn log(_: u32, _: *const u8, _: usize) {}

static STATE: StateApi = StateApi {
    version: 1,
    size: size_of::<StateApi>() as u32,
    register_region: None,
    info: None,
    read_at: None,
    write_batch: None,
    flush: None,
};

#[test]
fn read_copies_only_the_advertised_size() {
    assert!(unsafe { HostContext::read(core::ptr::null()) }.is_none());
    let mut full = HostContext::from_prefix(HostContextV1 {
        version: 1,
        size: size_of::<HostContext>() as u32,
        game: 1,
        phase: PHASE_LATE,
        game_base: 0x40_0000,
        game_size: 0x1000,
        log: Some(log),
        vfs_size: None,
        vfs_read: None,
        hook_create: None,
        hook_enable: None,
        hook_disable: None,
        hook_remove: None,
    });
    full.storage = Some(&STATE);
    let copy = unsafe { HostContext::read(&full) }.unwrap();
    assert!(copy.storage.is_some() && copy.covers(size_of::<HostContext>()));
    // A prefix-sized context: the tail reads as absent even though the
    // memory behind it holds a pointer.
    full.size = prefix_size() as u32;
    let copy = unsafe { HostContext::read(&full) }.unwrap();
    assert!(copy.storage.is_none() && copy.settings.is_none() && copy.save_events.is_none());
    assert!(!copy.covers(prefix_size() + 1));
    assert_eq!(copy.prefix().game_base, 0x40_0000);
    assert!(copy.prefix().log.is_some());
    // A size above the struct is clamped.
    full.size = 4096;
    assert_eq!(unsafe { HostContext::read(&full) }.unwrap().size, 4096);
}

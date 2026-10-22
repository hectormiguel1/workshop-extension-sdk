use super::*;
use crate::abi::{GAME_XIII, GAME_XIII_2, PHASE_LATE, SaveEventsApi, SettingsApi, StateApi};
use crate::testing::{self, FakeHost};

const XIII_LOG_VFS: Requirements = Requirements {
    game: Some(GAME_XIII),
    services: &[Service::Log, Service::VfsSize, Service::VfsRead],
    image: false,
};

#[test]
fn requirements_are_checked_in_order() {
    let mut fake = FakeHost::new(GAME_XIII);
    let ok = fake.context_mut().prefix();
    assert_eq!(ok.require(&XIII_LOG_VFS), Ok(()));
    assert_eq!(ok.require(&Requirements::ANY), Ok(()));
    let mut c = ok;
    c.version = 2;
    c.size = 0;
    assert_eq!(c.require(&Requirements::ANY), Err(Unusable::Version(2)));
    c = ok;
    c.size = 48;
    assert_eq!(c.require(&Requirements::ANY), Err(Unusable::Size(48)));
    c = ok;
    c.game = GAME_XIII_2;
    c.log = None;
    assert_eq!(
        c.require(&XIII_LOG_VFS),
        Err(Unusable::Game {
            found: 2,
            required: 1
        })
    );
    c = ok;
    c.vfs_size = None;
    c.vfs_read = None;
    assert_eq!(
        c.require(&XIII_LOG_VFS),
        Err(Unusable::MissingService("vfs_size"))
    );
    c = ok;
    c.game_base = 0;
    let image = Requirements {
        image: true,
        ..Requirements::ANY
    };
    assert_eq!(c.require(&image), Err(Unusable::Image));
    c.game_base = 1;
    c.game_size = 0;
    assert_eq!(c.require(&image), Err(Unusable::Image));
    c = ok;
    c.hook_create = None;
    c.hook_enable = None;
    c.hook_disable = None;
    c.hook_remove = None;
    for s in Service::HOOKS {
        let needs = Requirements {
            services: core::slice::from_ref(match s {
                Service::HookCreate => &Service::HookCreate,
                Service::HookEnable => &Service::HookEnable,
                Service::HookDisable => &Service::HookDisable,
                _ => &Service::HookRemove,
            }),
            ..Requirements::ANY
        };
        assert_eq!(c.require(&needs), Err(Unusable::MissingService(s.name())));
    }
    c.log = None;
    let log = Requirements {
        services: &[Service::Log],
        ..Requirements::ANY
    };
    assert_eq!(c.require(&log), Err(Unusable::MissingService("log")));
}

#[test]
fn refusals_read_as_the_units_log_lines() {
    let texts = [
        Unusable::Null.to_string(),
        Unusable::Version(2).to_string(),
        Unusable::Size(4).to_string(),
        Unusable::Game {
            found: 3,
            required: 1,
        }
        .to_string(),
        Unusable::MissingService("log").to_string(),
        Unusable::Image.to_string(),
    ];
    assert_eq!(
        texts,
        [
            "host context is null",
            "host ABI version 2 (need 1)",
            "host context size 4 is too small",
            "game 3 is not FINAL FANTASY XIII",
            "host service log is missing",
            "host context has no game image",
        ]
    );
    let services = [
        ServiceError::StorageVersion,
        ServiceError::StorageIncomplete,
        ServiceError::SettingsVersion,
        ServiceError::SaveEventsVersion,
    ]
    .map(|e| e.to_string());
    assert_eq!(
        services,
        [
            "storage service version/size",
            "storage service table incomplete",
            "settings service version/size",
            "save events service version/size",
        ]
    );
}

#[test]
fn files_are_read_bounded_and_statuses_reported() {
    let mut fake = FakeHost::new(GAME_XIII);
    FakeHost::add_file("vfx/x.xfv", b"package");
    FakeHost::add_file("empty", b"");
    let c = fake.context_mut().prefix();
    assert_eq!(c.read_file("vfx/x.xfv", 64).unwrap(), b"package");
    assert_eq!(c.read_file("empty", 64).unwrap(), b"");
    assert_eq!(c.file_size("vfx/x.xfv"), Ok(7));
    assert_eq!(c.read_file("vfx/x.xfv", 3), Err(ReadError::TooLarge(7)));
    assert_eq!(
        c.read_file("missing", 64),
        Err(ReadError::Status(status::NOT_FOUND))
    );
    let mut none = c;
    none.vfs_read = None;
    assert_eq!(none.read_file("vfx/x.xfv", 64), Err(ReadError::Unavailable));
    none.vfs_size = None;
    assert_eq!(none.file_size("vfx/x.xfv"), Err(ReadError::Unavailable));
    // A host whose read fails after a good size.
    unsafe extern "C" fn short(_: *const u8, _: usize, _: *mut u8, _: usize, w: *mut usize) -> i32 {
        unsafe { w.write(0) };
        status::SHORT_BUFFER
    }
    let mut failing = c;
    failing.vfs_read = Some(short);
    assert_eq!(
        failing.read_file("vfx/x.xfv", 64),
        Err(ReadError::Status(-3))
    );
    c.log_line(level::WARN, b"hello");
    none.log = None;
    none.log_line(level::WARN, b"dropped");
    assert_eq!(FakeHost::lines(), [(level::WARN, "hello".into())]);
}

#[test]
fn host_validates_before_reading_and_exposes_the_context() {
    let mut fake = FakeHost::new(GAME_XIII);
    assert_eq!(
        unsafe { Host::from_raw(core::ptr::null()) }.unwrap_err(),
        Unusable::Null
    );
    fake.context_mut().version = 9;
    assert_eq!(
        unsafe { Host::from_raw(fake.context()) }.unwrap_err(),
        Unusable::Version(9)
    );
    fake.context_mut().version = 1;
    fake.context_mut().size = 20;
    assert_eq!(
        unsafe { Host::from_raw(fake.context()) }.unwrap_err(),
        Unusable::Size(20)
    );
    fake.context_mut().size = size_of::<HostContext>() as u32;
    let host = unsafe { Host::from_raw(fake.context()) }.unwrap();
    assert_eq!(host.raw(), fake.context());
    assert_eq!((host.game(), host.phase()), (GAME_XIII, PHASE_LATE));
    assert_eq!(host.image().base, 0x40_0000);
    assert_eq!(host.context().size, 64.max(size_of::<HostContext>() as u32));
    assert_eq!(host.require(&XIII_LOG_VFS), Ok(()));
    assert!(host.hooks().is_some());
    FakeHost::add_file("a", b"xy");
    assert_eq!(
        (host.file_size("a"), host.read_file("a", 9)),
        (Ok(2), Ok(b"xy".to_vec()))
    );
    host.log(level::INFO, "plain");
    host.error(format_args!("e {}", 1));
    host.warn(format_args!("w {}", 2));
    host.info(format_args!("i {}", 3));
    assert_eq!(
        FakeHost::lines(),
        [
            (level::INFO, "plain".into()),
            (level::ERROR, "e 1".into()),
            (level::WARN, "w 2".into()),
            (level::INFO, "i 3".into()),
        ]
    );
    let slot = HostSlot::default();
    assert!(slot.get().is_none());
    slot.set(&host);
    assert_eq!(slot.get().unwrap().game(), GAME_XIII);
    slot.clear();
    assert!(HostSlot::new().get().is_none() && slot.get().is_none());
}

static BAD_STATE: StateApi = StateApi {
    version: 2,
    ..testing::STATE_API
};
static SHORT_STATE: StateApi = StateApi {
    size: 8,
    ..testing::STATE_API
};
static PARTIAL_STATE: StateApi = StateApi {
    read_at: None,
    ..testing::STATE_API
};
static BAD_SETTINGS: SettingsApi = SettingsApi {
    register_provider: None,
    ..testing::SETTINGS_API
};
static OLD_SETTINGS: SettingsApi = SettingsApi {
    version: 0,
    ..testing::SETTINGS_API
};
static SMALL_SETTINGS: SettingsApi = SettingsApi {
    size: 4,
    ..testing::SETTINGS_API
};
static BAD_SAVES: SaveEventsApi = SaveEventsApi {
    register_callback: None,
    ..testing::SAVE_EVENTS_API
};
static OLD_SAVES: SaveEventsApi = SaveEventsApi {
    version: 7,
    ..testing::SAVE_EVENTS_API
};
static SMALL_SAVES: SaveEventsApi = SaveEventsApi {
    size: 4,
    ..testing::SAVE_EVENTS_API
};

fn host(fake: &FakeHost) -> Host {
    unsafe { Host::from_raw(fake.context()) }.unwrap()
}

#[test]
fn service_tables_are_size_and_version_guarded() {
    let mut fake = FakeHost::new(GAME_XIII);
    let h = host(&fake);
    assert!(h.storage_api().unwrap().is_some());
    assert!(h.settings_api().unwrap().is_some());
    assert!(h.save_events_api().unwrap().is_some());
    // A prefix-only host: every tail reads as absent.
    fake.context_mut().size = prefix_size_x(&fake);
    let h = host(&fake);
    assert!(h.storage_api().unwrap().is_none());
    assert!(h.settings_api().unwrap().is_none());
    assert!(h.save_events_api().unwrap().is_none());
    fake.context_mut().size = size_of::<HostContext>() as u32;
    for (api, want) in [
        (&BAD_STATE, ServiceError::StorageVersion),
        (&SHORT_STATE, ServiceError::StorageVersion),
        (&PARTIAL_STATE, ServiceError::StorageIncomplete),
    ] {
        fake.context_mut().storage = Some(api);
        assert_eq!(host(&fake).storage_api().unwrap_err(), want);
    }
    for api in [&BAD_SETTINGS, &OLD_SETTINGS, &SMALL_SETTINGS] {
        fake.context_mut().settings = Some(api);
        assert_eq!(
            host(&fake).settings_api().unwrap_err(),
            ServiceError::SettingsVersion
        );
    }
    for api in [&BAD_SAVES, &OLD_SAVES, &SMALL_SAVES] {
        fake.context_mut().save_events = Some(api);
        assert_eq!(
            host(&fake).save_events_api().unwrap_err(),
            ServiceError::SaveEventsVersion
        );
    }
    fake.context_mut().storage = None;
    fake.context_mut().settings = None;
    fake.context_mut().save_events = None;
    let h = host(&fake);
    assert!(h.storage_api().unwrap().is_none() && h.settings_api().unwrap().is_none());
    assert!(h.save_events_api().unwrap().is_none());
    fake.context_mut().hook_remove = None;
    assert!(host(&fake).hooks().is_none());
}

fn prefix_size_x(_: &FakeHost) -> u32 {
    crate::abi::prefix_size() as u32
}

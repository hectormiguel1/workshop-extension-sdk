use super::*;
use crate::abi::GAME_XIII;
use crate::testing::{self, FakeHost};

fn region(id: &str) -> Region {
    Region::register(&testing::STATE_API, id).unwrap()
}

#[test]
fn ids_are_validated() {
    assert!(valid_id("example.ff13.zone_warp"));
    assert!(valid_id("a-b_c.9"));
    assert!(!valid_id(""));
    assert!(!valid_id(&"x".repeat(65)));
    assert!(!valid_id("no spaces"));
    let _host = FakeHost::new(GAME_XIII);
    assert_eq!(
        Region::register(&testing::STATE_API, "bad id").unwrap_err(),
        status::INVALID_ARGUMENT
    );
}

#[test]
fn regions_read_absent_until_written_then_round_trip() {
    let _host = FakeHost::new(GAME_XIII);
    let r = region("example.state");
    assert_eq!(r.handle(), 1);
    assert_eq!(
        Region::register(&testing::STATE_API, "example.state").unwrap_err(),
        status::STATE_CONFLICT
    );
    assert_eq!(r.info(), Ok(None));
    assert_eq!(r.read_all(), Ok(None));
    let info = r.write(0, 2, b"hello").unwrap();
    assert_eq!(
        info,
        RegionInfo {
            schema: 2,
            length: 5,
            revision: 1
        }
    );
    assert_eq!(r.info(), Ok(Some(info)));
    assert_eq!(r.read_all(), Ok(Some((info, b"hello".to_vec()))));
    let mut two = [0u8; 2];
    assert_eq!(r.read(3, &mut two), Ok(()));
    assert_eq!(&two, b"lo");
    assert_eq!(r.read(4, &mut two), Err(status::INVALID_ARGUMENT));
    // Stale revision, ranged batch, shrink to empty.
    assert_eq!(r.write(0, 2, b"x"), Err(status::STATE_CONFLICT));
    let info = r.write_ranges(1, 6, 3, &[(0, b"J"), (5, b"!")]).unwrap();
    assert_eq!(r.read_all().unwrap().unwrap().1, b"Jello!");
    let info = r.write(info.revision, 3, b"").unwrap();
    assert_eq!(info.length, 0);
    assert_eq!(r.read_all().unwrap().unwrap().1, b"");
    let too_many = vec![(0u64, &b""[..]); abi::STATE_MAX_RANGES + 1];
    assert_eq!(
        r.write_ranges(info.revision, 0, 3, &too_many),
        Err(status::STATE_LIMIT)
    );
    assert_eq!(
        r.write(info.revision, 0, b"a"),
        Err(status::INVALID_ARGUMENT)
    );
    assert_eq!(r.flush(), Ok(()));
    assert_eq!(FakeHost::flushes(), 1);
    assert_eq!(
        RegionInfo::from(StateInfo::default()),
        RegionInfo::default()
    );
}

#[test]
fn host_failures_are_reported() {
    let _host = FakeHost::new(GAME_XIII);
    let r = region("example.state");
    FakeHost::fail_services(
        status::STATE_RECOVERY_REQUIRED,
        status::STATE_IO_ERROR,
        0,
        0,
    );
    assert_eq!(
        Region::register(&testing::STATE_API, "other").unwrap_err(),
        status::STATE_RECOVERY_REQUIRED
    );
    assert_eq!(r.write(0, 1, b"a"), Err(status::STATE_IO_ERROR));
    FakeHost::fail_services(0, 0, 0, 0);
    r.write(0, 1, b"a").unwrap();
    FakeHost::info_size(4);
    assert_eq!(r.info(), Err(status::DECODE_FAILED));
    // A handle the host never issued, and tables without entries.
    let ghost = Region {
        api: testing::STATE_API,
        handle: 99,
    };
    assert_eq!(ghost.info(), Err(status::INVALID_ARGUMENT));
    assert_eq!(ghost.write(0, 1, b"a"), Err(status::INVALID_ARGUMENT));
    let empty = Region {
        api: StateApi {
            register_region: None,
            info: None,
            read_at: None,
            write_batch: None,
            flush: None,
            ..testing::STATE_API
        },
        handle: 1,
    };
    assert_eq!(empty.info(), Err(status::INVALID_ARGUMENT));
    assert_eq!(empty.read(0, &mut []), Err(status::INVALID_ARGUMENT));
    assert_eq!(empty.write(0, 1, b""), Err(status::INVALID_ARGUMENT));
    assert_eq!(empty.flush(), Ok(()));
    static NO_REGISTER: StateApi = StateApi {
        register_region: None,
        ..testing::STATE_API
    };
    assert_eq!(
        Region::register(&NO_REGISTER, "x").unwrap_err(),
        status::INVALID_ARGUMENT
    );
    unsafe extern "C" fn zero(_: *const u8, _: usize, h: *mut u32) -> i32 {
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
    static ZERO: StateApi = StateApi {
        register_region: Some(zero),
        flush: Some(failing_flush),
        info: Some(huge),
        ..testing::STATE_API
    };
    assert_eq!(
        Region::register(&ZERO, "x").unwrap_err(),
        status::INVALID_ARGUMENT
    );
    let odd = Region {
        api: ZERO,
        handle: 1,
    };
    assert_eq!(odd.flush(), Err(status::STATE_IO_ERROR));
    assert_eq!(odd.read_all(), Err(status::STATE_LIMIT));
}

#[test]
fn raw_registration_copies_the_table() {
    let _host = FakeHost::new(GAME_XIII);
    let table = testing::STATE_API;
    let r = unsafe { Region::register_raw(&table, "example.raw") }.unwrap();
    assert_eq!(r.write(0, 1, b"ok").unwrap().revision, 1);
}

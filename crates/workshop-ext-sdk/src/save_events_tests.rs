use super::*;
use crate::testing::{self, FakeHost, save_event};
use std::sync::Mutex;

static SEEN: Mutex<Vec<(u32, Option<u32>)>> = Mutex::new(Vec::new());

fn record(e: &SaveEvent) -> i32 {
    SEEN.lock().unwrap().push((e.kind, e.valid_slot()));
    0
}

fn boom(_: &SaveEvent) -> i32 {
    panic!("handler bug")
}

#[test]
fn kinds_and_masks() {
    for (i, k) in EventKind::ALL.iter().enumerate() {
        assert_eq!(k.raw(), i as u32 + 1);
        assert_eq!(EventKind::from_raw(k.raw()), Some(*k));
        assert_eq!(k.mask(), 1 << i);
    }
    assert_eq!(EventKind::from_raw(0), None);
    assert_eq!(mask_of(&EventKind::ALL), abi::SAVE_ALL_EVENTS);
    assert_eq!(mask_of(&[EventKind::Saved, EventKind::NewGame]), 0b1_0010);
    let mut e = save_event(abi::LOADED, Some(3));
    assert_eq!(
        (e.event_kind(), e.valid_slot()),
        (Some(EventKind::Loaded), Some(3))
    );
    assert_eq!(e.valid_profile(), None);
    e.flags |= abi::SAVE_PROFILE_VALID;
    assert_eq!(e.valid_profile(), Some(&[0; 32]));
    e.flags = 0;
    assert_eq!(e.valid_slot(), None);
    let new_game = save_event(abi::NEW_GAME, None);
    assert_eq!(new_game.valid_slot(), None);
    let mut bad = save_event(abi::SAVED, Some(1));
    bad.slot = abi::SAVE_NO_SLOT;
    assert_eq!(bad.valid_slot(), None);
}

#[test]
fn handlers_receive_events_and_panics_are_contained() {
    let _host = FakeHost::new(abi::GAME_XIII);
    let events = SaveEvents::new(&testing::SAVE_EVENTS_API);
    let info = events.info().unwrap();
    assert_eq!((info.events, info.slot_count), (abi::SAVE_ALL_EVENTS, 100));
    let mask = mask_of(&[EventKind::Saved, EventKind::NewGame]);
    assert_eq!(events.register("example.saves", mask, record), Ok(()));
    assert_eq!(
        events.register("example.boom", EventKind::Loaded.mask(), boom),
        Ok(())
    );
    assert_eq!(FakeHost::fire(abi::SAVED, Some(2)), [0]);
    assert_eq!(FakeHost::fire(abi::NEW_GAME, None), [0]);
    assert_eq!(FakeHost::fire(abi::SAVE_BEGIN, Some(2)), Vec::<i32>::new());
    assert_eq!(FakeHost::fire(abi::LOADED, Some(1)), [HANDLER_PANICKED]);
    assert_eq!(
        *SEEN.lock().unwrap(),
        [(abi::SAVED, Some(2)), (abi::NEW_GAME, None)]
    );
    // The trampoline refuses null and short events.
    let callbacks = FakeHost::save_callbacks();
    let (cb, user) = (callbacks[0].1.callback, callbacks[0].1.user);
    assert_eq!(
        unsafe { cb(core::ptr::null(), user) },
        status::INVALID_ARGUMENT
    );
    let mut short = save_event(abi::SAVED, Some(1));
    short.size = 8;
    assert_eq!(unsafe { cb(&short, user) }, status::INVALID_ARGUMENT);
    // Registration arguments and host failures.
    assert_eq!(
        events.register("x", 0, record),
        Err(status::INVALID_ARGUMENT)
    );
    assert_eq!(
        events.register("x", 64, record),
        Err(status::INVALID_ARGUMENT)
    );
    assert_eq!(
        events.register("", 1, record),
        Err(status::INVALID_ARGUMENT)
    );
    FakeHost::fail_services(0, 0, 0, status::HOOK_FAILED);
    assert_eq!(events.register("x", 1, record), Err(status::HOOK_FAILED));
    static EMPTY: SaveEventsApi = SaveEventsApi {
        info: None,
        register_callback: None,
        ..testing::SAVE_EVENTS_API
    };
    let empty = SaveEvents::new(&EMPTY);
    assert_eq!(empty.info(), Err(status::UNAVAILABLE));
    assert_eq!(empty.register("x", 1, record), Err(status::UNAVAILABLE));
    unsafe extern "C" fn failing(_: *mut SaveEventInfo) -> i32 {
        status::UNAVAILABLE
    }
    static FAILING: SaveEventsApi = SaveEventsApi {
        info: Some(failing),
        ..testing::SAVE_EVENTS_API
    };
    assert_eq!(SaveEvents::new(&FAILING).info(), Err(status::UNAVAILABLE));
}

#[test]
fn raw_services_copy_the_table() {
    let _host = FakeHost::new(abi::GAME_XIII);
    let table = testing::SAVE_EVENTS_API;
    let events = unsafe { SaveEvents::from_raw(&table) };
    assert_eq!(events.register("example.raw", 1, record), Ok(()));
}

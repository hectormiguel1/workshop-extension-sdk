use super::*;
use crate::abi::{self, GAME_XIII, HostContextV1, status};
use crate::hooks::HookStage;
use crate::testing::{self, FakeHost};
use core::sync::atomic::{AtomicUsize, Ordering};

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
    crate::ff13_unit! { init: start, shutdown: stop }
}

mod exports_without_shutdown {
    use super::*;
    crate::ff13_unit! { init: start }
}

mod prefix_exports {
    use super::*;
    crate::ff13_unit! { init: start, shutdown: stop, context: crate::abi::HostContextV1 }
}

mod prefix_exports_without_shutdown {
    use super::*;
    crate::ff13_unit! { init: start, context: HostContextV1 }
}

#[test]
fn the_exports_validate_run_and_shut_down() {
    let mut fake = FakeHost::new(GAME_XIII);
    assert_eq!(unsafe { exports::init(fake.context()) }, code::OK);
    assert!(FakeHost::logged("started on 1"));
    assert_eq!(
        unsafe { exports::init(core::ptr::null()) },
        code::UNUSABLE_CONTEXT
    );
    fake.context_mut().version = 2;
    fake.context_mut().log = None;
    let before = STARTED.load(Ordering::SeqCst);
    assert_eq!(
        unsafe { exports::init(fake.context()) },
        code::UNUSABLE_CONTEXT
    );
    fake.context_mut().version = 1;
    fake.context_mut().size = 8;
    assert_eq!(
        unsafe { exports_without_shutdown::init(fake.context()) },
        code::UNUSABLE_CONTEXT
    );
    assert_eq!(STARTED.load(Ordering::SeqCst), before);
    let prefix = HostContextV1 {
        size: abi::prefix_size() as u32,
        ..unsafe { *FakeHost::new(GAME_XIII).context() }.prefix()
    };
    assert_eq!(unsafe { prefix_exports::init(&prefix) }, code::OK);
    assert_eq!(
        unsafe { prefix_exports_without_shutdown::init(core::ptr::null()) },
        1
    );
    prefix_exports::shutdown();
    prefix_exports_without_shutdown::shutdown();
    exports::shutdown();
    exports_without_shutdown::shutdown();
    assert!(STOPPED.load(Ordering::SeqCst) >= 1);
    run_shutdown(|| panic!("shutdown bug"));
}

#[test]
fn errors_and_panics_become_host_codes() {
    let fake = FakeHost::new(GAME_XIII);
    let run = |f: fn(&Init) -> InitResult| unsafe { run_init(fake.context(), f) };
    assert_eq!(run(|_| Err(InitError::REFUSED)), code::REFUSED);
    assert_eq!(run(|_| Err(InitError::HOOK_FAILED)), code::HOOK_FAILED);
    assert_eq!(run(|_| Err(InitError::code(77))), 77);
    assert_eq!(run(|_| Err(InitError::code(0))), code::REFUSED);
    assert_eq!(run(|_| panic!("init bug {}", 7)), code::PANICKED);
    assert!(FakeHost::logged("unit panicked during init: init bug 7"));
    assert_eq!(run(|_| std::panic::panic_any(5u8)), code::PANICKED);
    assert!(FakeHost::logged("non-string payload"));
    assert_eq!(run(|_| panic!("static")), code::PANICKED);
    assert!(FakeHost::logged("init: static"));
    assert_eq!(InitError::UNUSABLE_CONTEXT.value(), 1);
    assert_eq!(InitError::REFUSED.to_string(), "init returned 2");
    let failure = crate::hooks::HookFailure {
        stage: HookStage::Create,
        target: 0,
        status: -6,
        removed: 0,
    };
    assert_eq!(InitError::from(failure), InitError::HOOK_FAILED);
    assert_eq!(
        InitError::from(ServiceError::SettingsVersion),
        InitError::REFUSED
    );
    // The prefix type is accepted as the context pointer too.
    let prefix: HostContextV1 = HostContextV1 {
        size: abi::PREFIX_SIZE_X86.max(abi::prefix_size() as u32),
        ..unsafe { *fake.context() }.prefix()
    };
    let none_registered = |init: &Init| {
        assert!(init.storage("x").unwrap().is_none());
        assert!(init.settings().unwrap().is_none());
        assert!(init.save_events().unwrap().is_none());
        Ok(())
    };
    assert_eq!(unsafe { run_init(&prefix, none_registered) }, 0);
}

static SLOT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn the_init_scope_hands_out_init_only_services() {
    let mut fake = FakeHost::new(GAME_XIII);
    let all = |init: &Init| {
        let region = init.storage("example.unit").unwrap().unwrap();
        region.write(0, 1, b"x").unwrap();
        init.settings()?.unwrap();
        let saves = init.save_events()?.unwrap();
        assert_eq!(
            saves.register("example.unit", abi::SAVE_ALL_EVENTS, |_| 0),
            Ok(())
        );
        let hooks = [crate::hooks::HookSpec {
            target: 0x40_1000,
            detour: 0x10,
            original: &SLOT,
        }];
        init.install_hooks(&hooks)?;
        assert!(init.hooks().is_some());
        Ok(())
    };
    assert_eq!(unsafe { run_init(fake.context(), all) }, 0);
    assert_eq!(FakeHost::hooks().len(), 1);
    assert_eq!(FakeHost::regions()[0].bytes, b"x");
    // A duplicate region and a malformed table.
    let dup = |init: &Init| {
        let err = init.storage("example.unit").unwrap_err();
        assert_eq!(err, StorageError::Register(status::STATE_CONFLICT));
        assert_eq!(err.to_string(), "storage region registration failed (-8)");
        Ok(())
    };
    assert_eq!(unsafe { run_init(fake.context(), dup) }, 0);
    static BAD: abi::StateApi = abi::StateApi {
        version: 9,
        ..testing::STATE_API
    };
    fake.context_mut().storage = Some(&BAD);
    let bad = |init: &Init| {
        let err = init.storage("x").unwrap_err();
        assert_eq!(err.to_string(), "storage service version/size");
        Err(match err {
            StorageError::Service(e) => e.into(),
            StorageError::Register(rc) => InitError::code(rc),
        })
    };
    assert_eq!(unsafe { run_init(fake.context(), bad) }, code::REFUSED);
    // Without MinHook services, installs fail at Create, nothing created.
    fake.context_mut().hook_create = None;
    let no_hooks = |init: &Init| {
        let spec = [crate::hooks::HookSpec {
            target: 0x40_2000,
            detour: 0x20,
            original: &SLOT,
        }];
        let failure = init.install_hooks(&spec).unwrap_err();
        assert_eq!(
            (failure.stage, failure.target, failure.status),
            (HookStage::Create, 0x40_2000, status::UNAVAILABLE)
        );
        assert_eq!(init.install_hooks(&[]).unwrap_err().target, 0);
        Err(failure.into())
    };
    assert_eq!(
        unsafe { run_init(fake.context(), no_hooks) },
        code::HOOK_FAILED
    );
}

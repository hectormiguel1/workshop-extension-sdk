use super::*;
use crate::abi::GAME_XIII;
use crate::testing::FakeHost;

static FIRST: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static SECOND: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];

fn specs(slots: &'static [AtomicUsize; 3]) -> [HookSpec<'static>; 3] {
    [0, 1, 2].map(|i| HookSpec {
        target: 0x40_0000 + 0x1000 * (i + 1),
        detour: 0x10 * (i + 1),
        original: &slots[i],
    })
}

fn factory(fake: &FakeHost) -> HookFactory {
    HookFactory::from_context(unsafe { &*fake.context() }).unwrap()
}

#[test]
fn a_set_installs_enabled_with_trampolines() {
    let fake = FakeHost::new(GAME_XIII);
    let f = factory(&fake);
    assert_eq!(f.install(&specs(&FIRST)), Ok(()));
    let hooks = FakeHost::hooks();
    assert_eq!(hooks.len(), 3);
    assert!(hooks.iter().all(|h| h.enabled));
    assert_eq!(FIRST[0].load(Ordering::Acquire), hooks[0].original);
    let control = f.control();
    assert_eq!(control.disable(0x40_1000), Ok(()));
    assert!(!FakeHost::hooks()[0].enabled);
    assert_eq!(control.enable(0x40_1000), Ok(()));
    assert_eq!(control.remove(0x40_1000), Ok(()));
    assert_eq!(control.remove(0x40_1000), Err(status::NOT_FOUND));
    assert_eq!(control.enable(0x40_1000), Err(status::NOT_FOUND));
    assert_eq!(control.disable(0x40_1000), Err(status::NOT_FOUND));
    let mut ctx = unsafe { *fake.context() };
    ctx.hook_create = None;
    assert!(HookFactory::from_context(&ctx).is_none());
    assert!(HookControl::from_context(&ctx).is_some());
}

#[test]
fn a_failed_create_or_enable_rolls_everything_back() {
    let fake = FakeHost::new(GAME_XIII);
    FakeHost::fail_hook_create_at(2);
    let failure = factory(&fake).install(&specs(&SECOND)).unwrap_err();
    assert_eq!(
        failure,
        HookFailure {
            stage: HookStage::Create,
            target: 0x40_3000,
            status: status::HOOK_FAILED,
            removed: 2
        }
    );
    assert_eq!(
        failure.to_string(),
        "hook create failed at 0x403000 (status -6); removed 2 hook(s)"
    );
    assert!(FakeHost::hooks().is_empty());
    assert_eq!(FakeHost::hook_cleanups(), (2, 2));
    assert_eq!(
        (
            SECOND[0].load(Ordering::Acquire),
            SECOND[1].load(Ordering::Acquire)
        ),
        (0, 0)
    );

    let fake = FakeHost::new(GAME_XIII);
    FakeHost::fail_hook_enable_at(1);
    let failure = factory(&fake).install(&specs(&SECOND)).unwrap_err();
    assert_eq!(
        (failure.stage, failure.target, failure.removed),
        (HookStage::Enable, 0x40_2000, 3)
    );
    assert_eq!(HookStage::Enable.text(), "enable");
    assert!(FakeHost::hooks().is_empty());

    let fake = FakeHost::new(GAME_XIII);
    FakeHost::null_trampolines();
    let failure = factory(&fake).install(&specs(&SECOND)).unwrap_err();
    assert_eq!((failure.status, failure.removed), (status::HOOK_FAILED, 0));
    assert_eq!(factory(&fake).create(1, 2), Err(status::HOOK_FAILED));
}

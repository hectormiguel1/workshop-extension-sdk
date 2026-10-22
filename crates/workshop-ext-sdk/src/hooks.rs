//! The host's MinHook services.
//!
//! Hooks are *created* only on the thread running the unit's `init`
//! ([`crate::unit::Init::hooks`]), so a failed init can clean up; they can
//! be enabled, disabled and removed later through [`HookControl`]. The host
//! forbids null/all-hooks targets and its own targets.
//!
//! [`install`] creates a whole set, enables it, and rolls everything back
//! on the first failure: a unit is either fully hooked or not at all.
use crate::abi::{HookCreateFn, HookTargetFn, HostContext, status};
use core::ffi::c_void;
use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Enable/disable/remove for hooks the unit created. Valid for the process
/// lifetime; copy it into hook paths freely.
#[derive(Clone, Copy, Debug)]
pub struct HookControl {
    enable: HookTargetFn,
    disable: HookTargetFn,
    remove: HookTargetFn,
}

fn status_of(rc: i32) -> Result<(), i32> {
    match rc {
        status::OK => Ok(()),
        rc => Err(rc),
    }
}

impl HookControl {
    /// All three services of `context`, or `None` when one is missing.
    pub fn from_context(context: &HostContext) -> Option<Self> {
        Some(Self {
            enable: context.hook_enable?,
            disable: context.hook_disable?,
            remove: context.hook_remove?,
        })
    }

    /// Enable the hook at `target`.
    pub fn enable(&self, target: usize) -> Result<(), i32> {
        // SAFETY: host contract; the host rejects targets the unit does not own.
        status_of(unsafe { (self.enable)(target as *mut c_void) })
    }

    /// Disable the hook at `target` (harmless when already disabled).
    pub fn disable(&self, target: usize) -> Result<(), i32> {
        // SAFETY: as `enable`.
        status_of(unsafe { (self.disable)(target as *mut c_void) })
    }

    /// Remove the hook at `target`.
    pub fn remove(&self, target: usize) -> Result<(), i32> {
        // SAFETY: as `enable`.
        status_of(unsafe { (self.remove)(target as *mut c_void) })
    }
}

/// Hook creation plus [`HookControl`]; only [`crate::unit::Init`] hands it out.
#[derive(Clone, Copy, Debug)]
pub struct HookFactory {
    create: HookCreateFn,
    control: HookControl,
}

impl HookFactory {
    /// The four services of `context`, or `None` when one is missing.
    ///
    /// Crate-private: only the init scope may create hooks.
    pub(crate) fn from_context(context: &HostContext) -> Option<Self> {
        Some(Self {
            create: context.hook_create?,
            control: HookControl::from_context(context)?,
        })
    }

    /// The enable/disable/remove services.
    pub const fn control(&self) -> HookControl {
        self.control
    }

    /// Create a disabled hook from `target` to `detour`; returns the
    /// trampoline (call it to run the original code).
    pub fn create(&self, target: usize, detour: usize) -> Result<usize, i32> {
        let mut original: *mut c_void = core::ptr::null_mut();
        // SAFETY: host contract; `original` is writable. The caller passes
        // a verified function entry and a detour of the same convention.
        let rc =
            unsafe { (self.create)(target as *mut c_void, detour as *mut c_void, &mut original) };
        match (rc, original.is_null()) {
            (status::OK, false) => Ok(original as usize),
            (status::OK, true) => Err(status::HOOK_FAILED),
            (rc, _) => Err(rc),
        }
    }

    /// [`install`] with these services.
    pub fn install(&self, hooks: &[HookSpec<'_>]) -> Result<(), HookFailure> {
        install(self, hooks)
    }
}

/// One hook of a set: `original` receives the trampoline (0 until created
/// and again after a rollback), which is what the detour calls.
#[derive(Clone, Copy, Debug)]
pub struct HookSpec<'a> {
    /// Verified function entry.
    pub target: usize,
    /// Detour with the target's calling convention.
    pub detour: usize,
    /// Trampoline slot read by the detour.
    pub original: &'a AtomicUsize,
}

/// Which step of [`install`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookStage {
    /// `hook_create` (or a null trampoline).
    Create,
    /// `hook_enable`.
    Enable,
}

impl HookStage {
    /// `"create"` or `"enable"`.
    pub const fn text(self) -> &'static str {
        match self {
            HookStage::Create => "create",
            HookStage::Enable => "enable",
        }
    }
}

/// A failed [`install`], after its rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookFailure {
    /// The failing step.
    pub stage: HookStage,
    /// Its target.
    pub target: usize,
    /// The host status.
    pub status: i32,
    /// Hooks that had been created and were disabled and removed again.
    pub removed: usize,
}

impl fmt::Display for HookFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "hook {} failed at {:#x} (status {}); removed {} hook(s)",
            self.stage.text(),
            self.target,
            self.status,
            self.removed
        )
    }
}

/// Create every hook (disabled), then enable every hook. On the first
/// failure every created hook is disabled and removed, its `original`
/// reset to 0, and nothing stays hooked.
pub fn install(factory: &HookFactory, hooks: &[HookSpec<'_>]) -> Result<(), HookFailure> {
    let mut created = 0;
    let mut failure = None;
    for spec in hooks {
        match factory.create(spec.target, spec.detour) {
            Ok(original) => {
                spec.original.store(original, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                failure = Some((HookStage::Create, spec.target, status));
                break;
            }
        }
    }
    if failure.is_none() {
        failure = hooks.iter().find_map(|spec| {
            factory
                .control
                .enable(spec.target)
                .err()
                .map(|status| (HookStage::Enable, spec.target, status))
        });
    }
    let Some((stage, target, status)) = failure else {
        return Ok(());
    };
    for spec in hooks.iter().take(created) {
        // Cleanup is best effort: the host also removes what a failed init
        // created before it unloads the unit.
        let _ = factory.control.disable(spec.target);
        let _ = factory.control.remove(spec.target);
        spec.original.store(0, Ordering::Release);
    }
    Err(HookFailure {
        stage,
        target,
        status,
        removed: created,
    })
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;

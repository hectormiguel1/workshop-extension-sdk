//! The unit boundary: the `init`/`shutdown` exports ([`ff13_unit!`](crate::ff13_unit!)), the
//! init scope ([`Init`]) and the host-expected return codes ([`code`]).
//!
//! An extension unit is an ordinary 32-bit Windows DLL exporting exactly
//! `init` and `shutdown` (cdecl). No `DllMain` logic, no proxy exports. The
//! host loads the unit, calls `init` once on its loader thread, keeps the
//! unit resident on success, and unloads it (removing the hooks it
//! created) on any nonzero return.
//!
//! [`run_init`] owns the shared boundary policy:
//!
//! * a null context, another ABI version or a context smaller than the
//!   version 1 prefix returns [`code::UNUSABLE_CONTEXT`] without reading
//!   any other field (not even `log`);
//! * the unit's function then runs with a validated [`Init`] scope;
//! * a panic is caught at the boundary (the host DLL profile unwinds) and
//!   returns [`code::PANICKED`] after logging it.
use crate::abi::HostContext;
use crate::hooks::{HookFactory, HookFailure, HookSpec};
use crate::host::{Host, ServiceError};
use crate::save_events::SaveEvents;
use crate::settings::Settings;
use crate::storage::Region;
use core::fmt;
use core::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// `init` return codes. The host treats every nonzero value as failure and
/// logs it (`init returned N`); these are the conventional ones.
pub mod code {
    /// Initialized.
    pub const OK: i32 = 0;
    /// The context cannot drive the unit (null, version, size, game, services).
    pub const UNUSABLE_CONTEXT: i32 = 1;
    /// The unit refused: the executable does not match its verified sites,
    /// or a service it needs failed.
    pub const REFUSED: i32 = 2;
    /// Creating or enabling a hook failed (everything was rolled back).
    pub const HOOK_FAILED: i32 = 3;
    /// The unit panicked during `init` (caught at the boundary).
    pub const PANICKED: i32 = 4;
}

/// A nonzero `init` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitError(i32);

impl InitError {
    /// [`code::UNUSABLE_CONTEXT`].
    pub const UNUSABLE_CONTEXT: InitError = InitError(code::UNUSABLE_CONTEXT);
    /// [`code::REFUSED`].
    pub const REFUSED: InitError = InitError(code::REFUSED);
    /// [`code::HOOK_FAILED`].
    pub const HOOK_FAILED: InitError = InitError(code::HOOK_FAILED);

    /// Any code; 0 becomes [`code::REFUSED`] (an error never reads as success).
    pub const fn code(code: i32) -> InitError {
        InitError(if code == 0 { self::code::REFUSED } else { code })
    }

    /// The value `init` returns.
    pub const fn value(self) -> i32 {
        self.0
    }
}

impl From<HookFailure> for InitError {
    fn from(_: HookFailure) -> Self {
        InitError::HOOK_FAILED
    }
}

impl From<ServiceError> for InitError {
    fn from(_: ServiceError) -> Self {
        InitError::REFUSED
    }
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "init returned {}", self.0)
    }
}

/// What a unit's init function returns.
pub type InitResult = Result<(), InitError>;

/// The init scope: the validated host plus the services the host allows
/// only on the thread running this unit's `init` (hook creation and the
/// storage/settings/save-event registrations). It cannot outlive the call.
pub struct Init<'a> {
    host: Host,
    _scope: PhantomData<&'a mut ()>,
}

impl Init<'_> {
    /// The validated host context (copy it freely; it outlives `init`).
    pub const fn host(&self) -> &Host {
        &self.host
    }

    /// Hook creation, when all four MinHook services are present.
    pub fn hooks(&self) -> Option<HookFactory> {
        HookFactory::from_context(self.host.context())
    }

    /// Create and enable a whole hook set, rolling back on failure
    /// ([`crate::hooks::install`]). Without the services the first hook
    /// fails at `Create` with `UNAVAILABLE`.
    pub fn install_hooks(&self, hooks: &[HookSpec<'_>]) -> Result<(), HookFailure> {
        match self.hooks() {
            Some(factory) => factory.install(hooks),
            None => Err(HookFailure {
                stage: crate::hooks::HookStage::Create,
                target: hooks.first().map_or(0, |h| h.target),
                status: crate::abi::status::UNAVAILABLE,
                removed: 0,
            }),
        }
    }

    /// Register the storage region `id` (1..=64 ASCII letters, digits,
    /// `._-`, stable across releases): `Ok(None)` when the host has no
    /// storage service.
    pub fn storage(&self, id: &str) -> Result<Option<Region>, StorageError> {
        match self.host.storage_api() {
            Err(e) => Err(StorageError::Service(e)),
            Ok(None) => Ok(None),
            Ok(Some(api)) => Region::register(api, id)
                .map(Some)
                .map_err(StorageError::Register),
        }
    }

    /// The settings registry: `Ok(None)` when the host has none.
    pub fn settings(&self) -> Result<Option<Settings>, ServiceError> {
        Ok(self.host.settings_api()?.map(Settings::new))
    }

    /// The save events service: `Ok(None)` when the host has none.
    pub fn save_events(&self) -> Result<Option<SaveEvents>, ServiceError> {
        Ok(self.host.save_events_api()?.map(SaveEvents::new))
    }
}

/// A failed [`Init::storage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// The advertised table is malformed.
    Service(ServiceError),
    /// `register_region` returned this status (`STATE_CONFLICT`: the ID is
    /// owned by another component).
    Register(i32),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Service(e) => e.fmt(f),
            StorageError::Register(rc) => write!(f, "storage region registration failed ({rc})"),
        }
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for crate::abi::HostContext {}
    impl Sealed for crate::abi::HostContextV1 {}
}

/// A context type `init` may be declared with: [`HostContext`] (the
/// default) or its version 1 prefix [`crate::abi::HostContextV1`]. Both
/// start with the same `#[repr(C)]` fields; the boundary reads only what
/// `size` advertises.
pub trait ContextLayout: sealed::Sealed {}
impl ContextLayout for HostContext {}
impl ContextLayout for crate::abi::HostContextV1 {}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string payload")
}

/// The `init` boundary: validate `context`, run `unit` in an [`Init`]
/// scope, catch panics, return the host code.
///
/// # Safety
/// `context` is null or a host context readable for its advertised
/// `size`, valid for the process lifetime.
pub unsafe fn run_init<C: ContextLayout>(
    context: *const C,
    unit: impl FnOnce(&Init<'_>) -> InitResult,
) -> i32 {
    // SAFETY: caller contract (`C` is `HostContext` or its prefix).
    let Ok(host) = (unsafe { Host::from_raw(context.cast::<HostContext>()) }) else {
        return code::UNUSABLE_CONTEXT;
    };
    let init = Init {
        host,
        _scope: PhantomData,
    };
    match catch_unwind(AssertUnwindSafe(|| unit(&init))) {
        Ok(Ok(())) => code::OK,
        Ok(Err(e)) => e.value(),
        Err(payload) => {
            host.error(format_args!(
                "unit panicked during init: {}",
                panic_text(&*payload)
            ));
            code::PANICKED
        }
    }
}

/// The `shutdown` boundary: run `unit`, swallowing a panic (it must never
/// unwind into the host).
pub fn run_shutdown(unit: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(unit));
}

/// Emits the unit's two exports, `init` and `shutdown` (cdecl), and nothing
/// else: no `DllMain`, no proxy exports.
///
/// ```no_run
/// fn start(init: &workshop_ext_sdk::unit::Init) -> workshop_ext_sdk::unit::InitResult {
///     init.host().info(format_args!("hello {}", init.host().game()));
///     Ok(())
/// }
/// fn stop() {}
/// workshop_ext_sdk::ff13_unit! { init: start, shutdown: stop }
/// ```
///
/// `init` validates the context ([`run_init`]) and calls the init
/// function with an [`Init`] scope; `shutdown` calls the shutdown function
/// (optional) with panics caught. `context: Type` declares `init`'s
/// parameter as `*const Type` ([`ContextLayout`]; default [`HostContext`]).
///
/// The functions are exported unmangled except in the invoking crate's own
/// `cfg(test)` builds, where several invocations may coexist and `init` /
/// `shutdown` would clash with each other or with the C library; tests call
/// them by path.
#[macro_export]
macro_rules! ff13_unit {
    (init: $init:path $(, shutdown: $shutdown:path)? $(,)?) => {
        $crate::ff13_unit! { init: $init, $(shutdown: $shutdown,)? context: $crate::abi::HostContext }
    };
    (init: $init:path, $(shutdown: $shutdown:path,)? context: $context:ty $(,)?) => {
        /// The unit's `init` export (called by Crux).
        ///
        /// # Safety
        /// Called by the host with null or its context (`ff13_host.h`).
        #[cfg_attr(not(test), unsafe(no_mangle))]
        pub unsafe extern "C" fn init(context: *const $context) -> i32 {
            // SAFETY: host contract.
            unsafe { $crate::unit::run_init(context, $init) }
        }

        /// The unit's `shutdown` export (called by Crux).
        #[cfg_attr(not(test), unsafe(no_mangle))]
        pub extern "C" fn shutdown() {
            $crate::unit::run_shutdown(|| { $( $shutdown(); )? });
        }
    };
}

#[cfg(test)]
#[path = "unit_tests.rs"]
mod tests;

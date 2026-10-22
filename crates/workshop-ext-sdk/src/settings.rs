//! Settings providers: the controls a unit shows in the game's shared
//! *Mod Settings* menu (`ff13_settings.h`, contract version 1).
//!
//! A provider is plain metadata the host's menu adapter reads: sections,
//! checkboxes, integer sliders, floats, combos, buttons, each with a stable
//! ID independent of its label or row. Labels and help are ZTR keys or
//! UTF-8. The descriptor is built once and lives for the process; callbacks
//! run on the menu's thread, outside host locks, and every one is guarded
//! against panics.
//!
//! Declare a provider with [`settings!`](crate::settings!), register its
//! `query` in `init` through [`crate::unit::Init::settings`]:
//!
//! ```
//! use core::sync::atomic::{AtomicBool, Ordering};
//! static LOUD: AtomicBool = AtomicBool::new(false);
//!
//! workshop_ext_sdk::settings! {
//!     /// The unit's menu.
//!     pub mod menu {
//!         id: "example.hello",
//!         name: "Hello",
//!         author: "Example",
//!         release: "1.0.0",
//!         help: "A minimal unit.",
//!         game: workshop_ext_sdk::abi::GAME_XIII,
//!         controls: |b| b
//!             .section("general", "General", "")
//!             .checkbox("loud", "Loud logging", "Log every frame.",
//!                 || LOUD.load(Ordering::Relaxed),
//!                 |on| LOUD.store(on, Ordering::Relaxed)),
//!     }
//! }
//!
//! fn main() {
//!     let provider = menu::descriptor();
//!     assert_eq!(provider.count, 2);
//! }
//! ```
use crate::abi::{
    self, SettingControl, SettingDisplayExtension, SettingExtension, SettingPresentationExtension,
    SettingStateExtension, SettingTextExtension, SettingsProvider, SettingsQueryContext,
};
use crate::param::Param;
use core::ffi::{c_char, c_void};
use std::borrow::Cow;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Status text row: legacy control kind 9, carried for parity with
/// existing providers. Not part of the v1 header; adapters may ignore it.
pub const SETTING_STATUS: u32 = 9;
/// Legacy "collapsed" presentation hint, carried for parity (adapters ignore it).
pub const SETTING_COLLAPSED: u32 = 8;
/// Longest status text a [`Builder::status`] row reports.
pub const STATUS_TEXT_MAX: usize = 2048;

/// Why a builder call is invalid. [`Builder`] panics with these messages:
/// a malformed descriptor is a bug in the unit, found by its tests.
pub mod error {
    /// More than 256 controls.
    pub const TOO_MANY: &str = "settings: more than 256 controls";
    /// Duplicate control or section ID.
    pub const DUPLICATE: &str = "settings: duplicate control id";
    /// A string is too long or holds a NUL byte.
    pub const STRING: &str = "settings: invalid string (too long or NUL)";
    /// The same extension kind twice on one control.
    pub const EXTENSION: &str = "settings: duplicate extension";
    /// An extension call before any control.
    pub const NO_CONTROL: &str = "settings: add a control first";
    /// Combo without options, or a bad numeric range.
    pub const RANGE: &str = "settings: invalid range or options";
}

fn leak<T>(value: T) -> *mut c_void {
    Box::into_raw(Box::new(value)) as *mut c_void
}

/// A NUL-terminated copy of `s` (at most `cap` bytes), valid for the process.
pub fn cstr(s: &str, cap: usize) -> *const c_char {
    assert!(
        s.len() <= cap && !s.as_bytes().contains(&0),
        "{}",
        error::STRING
    );
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    Box::leak(v.into_boxed_slice()).as_ptr().cast()
}

/// The text at a descriptor string pointer (`""` for null or non-UTF-8).
///
/// # Safety
/// `p` is null or a NUL-terminated string valid for `'static`.
pub unsafe fn text(p: *const c_char) -> &'static str {
    if p.is_null() {
        return "";
    }
    // SAFETY: caller contract.
    unsafe { core::ffi::CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
}

/// The controls of a provider.
///
/// # Safety
/// `provider` comes from [`Builder::finish`] (or is equally well formed).
pub unsafe fn controls(provider: &SettingsProvider) -> &'static [SettingControl] {
    if provider.controls.is_null() {
        return &[];
    }
    // SAFETY: caller contract: `count` controls at `controls`.
    unsafe { core::slice::from_raw_parts(provider.controls, provider.count as usize) }
}

type ReadF32 = Box<dyn Fn() -> f32 + Send + Sync>;
type WriteF32 = Box<dyn Fn(f32) + Send + Sync>;
type Action = Box<dyn Fn() + Send + Sync>;
type ReadU32 = Box<dyn Fn() -> u32 + Send + Sync>;
type ReadText = Box<dyn Fn() -> Cow<'static, str> + Send + Sync>;

struct Value {
    read: ReadF32,
    write: WriteF32,
}

unsafe extern "C" fn read_value(user: *mut c_void) -> f32 {
    // SAFETY: `user` is the `Value` this builder leaked.
    let v = unsafe { &*(user as *const Value) };
    catch_unwind(AssertUnwindSafe(|| (v.read)())).unwrap_or(0.0)
}

unsafe extern "C" fn write_value(user: *mut c_void, value: f32) {
    // SAFETY: `user` is the `Value` this builder leaked.
    let v = unsafe { &*(user as *const Value) };
    let _ = catch_unwind(AssertUnwindSafe(|| (v.write)(value)));
}

unsafe extern "C" fn activate(user: *mut c_void) {
    // SAFETY: `user` is the `Action` this builder leaked.
    let f = unsafe { &*(user as *const Action) };
    let _ = catch_unwind(AssertUnwindSafe(f));
}

unsafe extern "C" fn read_state(user: *mut c_void) -> u32 {
    // SAFETY: `user` is the `ReadU32` this builder leaked.
    let f = unsafe { &*(user as *const ReadU32) };
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(abi::SETTING_VISIBLE)
}

/// Copies the status text (UTF-8, NUL-terminated, never split inside a
/// char) into `output`; returns the full text length (at most 2048).
unsafe extern "C" fn read_text(user: *mut c_void, output: *mut u8, capacity: u32) -> u32 {
    // SAFETY: `user` is the `ReadText` this builder leaked.
    let f = unsafe { &*(user as *const ReadText) };
    let text = catch_unwind(AssertUnwindSafe(f)).unwrap_or(Cow::Borrowed(""));
    let length = text.len().min(STATUS_TEXT_MAX);
    if capacity != 0 && !output.is_null() {
        let mut n = length.min(capacity as usize - 1);
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        // SAFETY: the caller supplies `capacity` writable bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(text.as_ptr(), output, n);
            output.add(n).write(0);
        }
    }
    length as u32
}

/// Round a menu value to the nearest integer in `min..=max`; `None` for NaN
/// or infinity (the write is then ignored).
pub fn whole(value: f32, min: i64, max: i64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    Some((value.round() as i64).clamp(min, max))
}

/// Provider metadata for [`Builder::finish`].
#[derive(Clone, Copy, Debug)]
pub struct Meta<'a> {
    /// Provider ID (the registered ID; ≤ 64 bytes).
    pub id: &'a str,
    /// Display name (≤ 128 bytes).
    pub name: &'a str,
    /// Author (≤ 128 bytes).
    pub author: &'a str,
    /// Release text (≤ 128 bytes).
    pub release: &'a str,
    /// Help text (≤ 1024 bytes).
    pub help: &'a str,
    /// Website, if any (≤ 128 bytes).
    pub website: Option<&'a str>,
}

/// Builds a provider descriptor once; every string, callback and table is
/// leaked for the process lifetime, as the contract requires.
pub struct Builder {
    items: Vec<SettingControl>,
    ids: Vec<String>,
    extension_kinds: u32,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// An empty provider.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            ids: Vec::new(),
            extension_kinds: 0,
        }
    }

    /// Controls so far.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no control was added.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn push(&mut self, kind: u32, id: &str, label: &str, help: &str) -> &mut SettingControl {
        assert!(
            self.items.len() < abi::SETTINGS_MAX_CONTROLS,
            "{}",
            error::TOO_MANY
        );
        assert!(
            !self.ids.iter().any(|known| known == id),
            "{}",
            error::DUPLICATE
        );
        assert!(!id.is_empty(), "{}", error::STRING);
        self.ids.push(id.to_owned());
        self.extension_kinds = 0;
        self.items.push(SettingControl {
            size: size_of::<SettingControl>() as u32,
            kind,
            id: cstr(id, abi::SETTINGS_MAX_ID),
            label: cstr(label, abi::SETTINGS_MAX_LABEL),
            help: cstr(help, abi::SETTINGS_MAX_HELP),
            min: 0.0,
            max: 0.0,
            step: 0.0,
            read: None,
            write: None,
            user: core::ptr::null_mut(),
            activate: None,
            option_count: 0,
            options: core::ptr::null(),
            extensions: core::ptr::null(),
        });
        self.last()
    }

    fn last(&mut self) -> &mut SettingControl {
        self.items.last_mut().expect(error::NO_CONTROL)
    }

    /// A section: a new settings page/group until the next section.
    pub fn section(mut self, id: &str, label: &str, help: &str) -> Self {
        self.push(abi::SETTING_SECTION, id, label, help);
        self
    }

    /// A value control of any numeric `kind`, with raw `f32` callbacks
    /// (the typed methods below are built on this).
    #[allow(clippy::too_many_arguments)]
    pub fn value(
        mut self,
        kind: u32,
        id: &str,
        label: &str,
        help: &str,
        range: (f32, f32, f32),
        read: impl Fn() -> f32 + Send + Sync + 'static,
        write: impl Fn(f32) + Send + Sync + 'static,
    ) -> Self {
        let (min, max, step) = range;
        assert!(min <= max && step >= 0.0, "{}", error::RANGE);
        let c = self.push(kind, id, label, help);
        c.min = min;
        c.max = max;
        c.step = step;
        c.read = Some(read_value);
        c.write = Some(write_value);
        c.user = leak(Value {
            read: Box::new(read),
            write: Box::new(write),
        });
        self
    }

    /// A checkbox (0 or 1; a written value of 0.5 or more is `true`).
    pub fn checkbox(
        self,
        id: &str,
        label: &str,
        help: &str,
        read: impl Fn() -> bool + Send + Sync + 'static,
        write: impl Fn(bool) + Send + Sync + 'static,
    ) -> Self {
        self.value(
            abi::SETTING_CHECKBOX,
            id,
            label,
            help,
            (0.0, 1.0, 1.0),
            move || f32::from(u8::from(read())),
            move |v| {
                if v.is_finite() {
                    write(v >= 0.5)
                }
            },
        )
    }

    /// An integer slider over `min..=max` by `step` (writes are rounded and
    /// clamped; NaN/infinity ignored).
    #[allow(clippy::too_many_arguments)]
    pub fn integer(
        self,
        id: &str,
        label: &str,
        help: &str,
        (min, max, step): (i32, i32, i32),
        read: impl Fn() -> i32 + Send + Sync + 'static,
        write: impl Fn(i32) + Send + Sync + 'static,
    ) -> Self {
        assert!(step > 0, "{}", error::RANGE);
        self.value(
            abi::SETTING_INTEGER,
            id,
            label,
            help,
            (min as f32, max as f32, step as f32),
            move || read() as f32,
            move |v| {
                if let Some(v) = whole(v, min.into(), max.into()) {
                    write(v as i32)
                }
            },
        )
    }

    /// A float control over `min..=max` by `step` (writes clamped; NaN and
    /// infinity ignored).
    #[allow(clippy::too_many_arguments)]
    pub fn float(
        self,
        id: &str,
        label: &str,
        help: &str,
        (min, max, step): (f32, f32, f32),
        read: impl Fn() -> f32 + Send + Sync + 'static,
        write: impl Fn(f32) + Send + Sync + 'static,
    ) -> Self {
        self.value(
            abi::SETTING_FLOAT,
            id,
            label,
            help,
            (min, max, step),
            read,
            move |v| {
                if v.is_finite() {
                    write(v.clamp(min, max))
                }
            },
        )
    }

    /// A combo: option index `0..options.len()` (writes rounded and clamped).
    pub fn combo(
        self,
        id: &str,
        label: &str,
        help: &str,
        options: &[&str],
        read: impl Fn() -> u32 + Send + Sync + 'static,
        write: impl Fn(u32) + Send + Sync + 'static,
    ) -> Self {
        let last = options.len() as i64 - 1;
        self.value(
            abi::SETTING_COMBO,
            id,
            label,
            help,
            (0.0, last.max(0) as f32, 1.0),
            move || read() as f32,
            move |v| {
                if let Some(v) = whole(v, 0, last) {
                    write(v as u32)
                }
            },
        )
        .options(options)
    }

    /// The option labels of the last control (a combo; at least one and at
    /// most 256 labels).
    pub fn options(mut self, options: &[&str]) -> Self {
        assert!(
            !options.is_empty() && options.len() <= abi::SETTINGS_MAX_CONTROLS,
            "{}",
            error::RANGE
        );
        let strings: Vec<*const c_char> = options
            .iter()
            .map(|s| cstr(s, abi::SETTINGS_MAX_LABEL))
            .collect();
        let c = self.last();
        c.option_count = options.len() as u32;
        c.options = Box::leak(strings.into_boxed_slice()).as_ptr();
        self
    }

    fn param_value(
        self,
        kind: u32,
        (id, label, help): (&str, &str, &str),
        step: u32,
        param: &'static Param,
    ) -> Self {
        assert!(step > 0, "{}", error::RANGE);
        self.value(
            kind,
            id,
            label,
            help,
            (param.min as f32, param.max as f32, step as f32),
            move || param.get() as f32,
            move |v| {
                if let Some(v) = Param::from_menu(v) {
                    param.set(v);
                }
            },
        )
    }

    /// A checkbox bound to a 0/1 [`Param`]: menu writes go through
    /// [`Param::set`] (change callback, no disk write).
    pub fn param_checkbox(self, id: &str, label: &str, help: &str, param: &'static Param) -> Self {
        assert!(param.min == 0 && param.max == 1, "{}", error::RANGE);
        self.param_value(abi::SETTING_CHECKBOX, (id, label, help), 1, param)
    }

    /// An integer slider over a [`Param`]'s range by `step`.
    pub fn param_integer(
        self,
        id: &str,
        label: &str,
        help: &str,
        step: u32,
        param: &'static Param,
    ) -> Self {
        self.param_value(abi::SETTING_INTEGER, (id, label, help), step, param)
    }

    /// A combo over a [`Param`] whose range is `0..options.len()`.
    pub fn param_combo(
        self,
        id: &str,
        label: &str,
        help: &str,
        options: &[&str],
        param: &'static Param,
    ) -> Self {
        assert!(
            param.min == 0 && param.max as usize + 1 == options.len(),
            "{}",
            error::RANGE
        );
        self.param_value(abi::SETTING_COMBO, (id, label, help), 1, param)
            .options(options)
    }

    /// A button.
    pub fn button(
        mut self,
        id: &str,
        label: &str,
        help: &str,
        on_press: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let c = self.push(abi::SETTING_BUTTON, id, label, help);
        c.activate = Some(activate);
        c.user = leak(Box::new(on_press) as Action);
        self
    }

    /// A status text row ([`SETTING_STATUS`] with the text extension,
    /// refreshed every 250 ms).
    pub fn status(
        mut self,
        id: &str,
        label: &str,
        text: impl Fn() -> Cow<'static, str> + Send + Sync + 'static,
    ) -> Self {
        self.push(SETTING_STATUS, id, label, "");
        let header = self.extension::<SettingTextExtension>(abi::SETTING_EXT_TEXT);
        let user = leak(Box::new(text) as ReadText);
        self.last().extensions = leak(SettingTextExtension {
            header,
            read_text: Some(read_text),
            user,
            refresh_ms: 250,
        }) as *const SettingExtension;
        self
    }

    fn extension<T>(&mut self, kind: u32) -> SettingExtension {
        let next = self.last().extensions;
        let bit = 1 << (kind - abi::SETTING_EXT_STATE);
        assert!(self.extension_kinds & bit == 0, "{}", error::EXTENSION);
        self.extension_kinds |= bit;
        SettingExtension {
            size: size_of::<T>() as u32,
            kind,
            version: 1,
            next,
        }
    }

    /// Visible/enabled/busy state of the last control
    /// (`SETTING_VISIBLE | SETTING_ENABLED | SETTING_BUSY`).
    pub fn state(mut self, f: impl Fn() -> u32 + Send + Sync + 'static) -> Self {
        let header = self.extension::<SettingStateExtension>(abi::SETTING_EXT_STATE);
        let user = leak(Box::new(f) as ReadU32);
        self.last().extensions = leak(SettingStateExtension {
            header,
            read_state: Some(read_state),
            user,
        }) as *const SettingExtension;
        self
    }

    /// Presentation flags of the last control or section
    /// (`SETTING_ADVANCED | SETTING_RESTART_REQUIRED | SETTING_EXPERIMENTAL`).
    pub fn presentation(mut self, flags: u32) -> Self {
        let header = self.extension::<SettingPresentationExtension>(abi::SETTING_EXT_PRESENTATION);
        self.last().extensions =
            leak(SettingPresentationExtension { header, flags }) as *const SettingExtension;
        self
    }

    /// Prefix, suffix (≤ 24 bytes) and decimals (0..=6) of the last numeric control.
    pub fn display(mut self, prefix: &str, suffix: &str, decimals: u32) -> Self {
        assert!(decimals <= 6, "{}", error::RANGE);
        let header = self.extension::<SettingDisplayExtension>(abi::SETTING_EXT_DISPLAY);
        let (prefix, suffix) = (
            cstr(prefix, abi::SETTINGS_MAX_AFFIX),
            cstr(suffix, abi::SETTINGS_MAX_AFFIX),
        );
        self.last().extensions = leak(SettingDisplayExtension {
            header,
            prefix,
            suffix,
            decimals,
        }) as *const SettingExtension;
        self
    }

    /// The finished descriptor, leaked for the process lifetime.
    pub fn finish(self, meta: &Meta<'_>) -> &'static SettingsProvider {
        let count = self.items.len() as u32;
        Box::leak(Box::new(SettingsProvider {
            size: size_of::<SettingsProvider>() as u32,
            version: abi::SETTINGS_VERSION,
            id: cstr(meta.id, abi::SETTINGS_MAX_ID),
            name: cstr(meta.name, abi::SETTINGS_MAX_NAME),
            count,
            controls: Box::leak(self.items.into_boxed_slice()).as_ptr(),
            reserved_menu_count: 0,
            reserved_menu_stride: 24,
            reserved_menus: core::ptr::null(),
            control_stride: size_of::<SettingControl>() as u32,
            author: cstr(meta.author, abi::SETTINGS_MAX_NAME),
            release: cstr(meta.release, abi::SETTINGS_MAX_NAME),
            help: cstr(meta.help, abi::SETTINGS_MAX_HELP),
            website: meta
                .website
                .map_or(core::ptr::null(), |w| cstr(w, abi::SETTINGS_MAX_NAME)),
            reserved_color: [0.0; 4],
            reserved_flags: 0,
        }))
    }
}

/// Every string a menu adapter looks up for `provider`, in descriptor
/// order and without repeats or empty strings: the provider name, each
/// section's and control's label and help, combo options and display
/// prefix/suffix. Status rows are left out (the XIII menu adapter does not
/// show them). Units that ship their own ZTR translations check their
/// catalog against this list.
///
/// # Safety
/// `provider` comes from [`Builder::finish`] (or is equally well formed).
pub unsafe fn menu_strings(provider: &SettingsProvider) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut add = |s: &'static str| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    // SAFETY: caller contract: a well-formed provider, strings and tables
    // valid for the process.
    unsafe {
        add(text(provider.name));
        for c in controls(provider) {
            if c.kind == SETTING_STATUS {
                continue;
            }
            add(text(c.label));
            add(text(c.help));
            for i in 0..c.option_count as usize {
                add(text(*c.options.add(i)));
            }
            let mut ext = c.extensions;
            while let Some(e) = ext.as_ref() {
                if e.kind == abi::SETTING_EXT_DISPLAY {
                    let d = &*(ext as *const SettingDisplayExtension);
                    add(text(d.prefix));
                    add(text(d.suffix));
                }
                ext = e.next;
            }
        }
    }
    out
}

/// Whether a query context is one this provider answers: settings
/// version 1, a complete context, and `game` (0 = any game).
pub fn accepts(context: Option<&SettingsQueryContext>, game: u32) -> bool {
    context.is_some_and(|c| {
        c.size as usize >= size_of::<SettingsQueryContext>()
            && c.version == abi::SETTINGS_VERSION
            && (game == 0 || c.game == game)
    })
}

/// The body of a generated `query`: the descriptor (built once by `build`)
/// when [`accepts`], else null. A panicking builder answers null.
///
/// # Safety
/// `context` is null or a readable query context.
pub unsafe fn answer(
    context: *const SettingsQueryContext,
    game: u32,
    build: fn() -> &'static SettingsProvider,
) -> *const SettingsProvider {
    // SAFETY: caller contract.
    if !accepts(unsafe { context.as_ref() }, game) {
        return core::ptr::null();
    }
    catch_unwind(build).map_or(core::ptr::null(), |p| p as *const SettingsProvider)
}

/// Declares a settings provider module: `ID`, `descriptor()` (built once)
/// and the `query` callback to register in `init`. Metadata fields are
/// expressions (`website` is optional); `game` is the game the provider
/// answers (0 = any); `controls` maps a [`Builder`] to the finished list.
#[macro_export]
macro_rules! settings {
    (
        $(#[$meta:meta])*
        $vis:vis mod $name:ident {
            id: $id:expr,
            name: $title:expr,
            author: $author:expr,
            release: $release:expr,
            help: $help:expr,
            $(website: $website:expr,)?
            game: $game:expr,
            controls: $controls:expr $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis mod $name {
            // The user's expressions are evaluated in the invoking module's
            // scope (glob import), apart from this module's own names.
            mod __ff13_user {
                #[allow(unused_imports)]
                use super::super::*;

                pub(super) const __FF13_ID: &str = $id;
                pub(super) const __FF13_GAME: u32 = $game;

                pub(super) fn __ff13_build() -> &'static $crate::abi::SettingsProvider {
                    let build: fn($crate::settings::Builder) -> $crate::settings::Builder =
                        $controls;
                    build($crate::settings::Builder::new()).finish(&$crate::settings::Meta {
                        id: __FF13_ID,
                        name: $title,
                        author: $author,
                        release: $release,
                        help: $help,
                        website: $crate::__settings_website!($($website)?),
                    })
                }
            }

            /// The provider ID (register the query under it).
            #[allow(dead_code)]
            pub const ID: &str = __ff13_user::__FF13_ID;

            /// The descriptor, built on first use and kept for the process.
            pub fn descriptor() -> &'static $crate::abi::SettingsProvider {
                static CELL: ::std::sync::OnceLock<usize> = ::std::sync::OnceLock::new();
                let address = *CELL.get_or_init(|| {
                    __ff13_user::__ff13_build() as *const $crate::abi::SettingsProvider as usize
                });
                // SAFETY: the address of the leaked, never-freed descriptor.
                unsafe { &*(address as *const $crate::abi::SettingsProvider) }
            }

            /// The `FF13SettingsQuery` callback.
            ///
            /// # Safety
            /// Called by the host with null or a readable query context.
            pub unsafe extern "C" fn query(
                context: *const $crate::abi::SettingsQueryContext,
            ) -> *const $crate::abi::SettingsProvider {
                // SAFETY: host contract.
                unsafe { $crate::settings::answer(context, __ff13_user::__FF13_GAME, descriptor) }
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __settings_website {
    () => {
        None
    };
    ($website:expr) => {
        Some($website)
    };
}

/// The settings registry of an init scope.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    api: abi::SettingsApi,
}

impl Settings {
    pub(crate) const fn new(api: &abi::SettingsApi) -> Self {
        Self { api: *api }
    }

    /// The registry of a raw table. Prefer [`crate::unit::Init::settings`],
    /// which validates the table and enforces the init scope.
    ///
    /// # Safety
    /// Register only on the thread running the unit's `init` (host
    /// contract), with a table the host advertised.
    pub const unsafe fn from_raw(api: &abi::SettingsApi) -> Self {
        Self { api: *api }
    }

    /// Register `query` under the stable provider `id`. The provider
    /// appears after `init` succeeds; `STATE_CONFLICT` when another module
    /// owns the ID.
    pub fn register(&self, id: &str, query: abi::SettingsQueryFn) -> Result<(), i32> {
        let register = self.api.register_provider.ok_or(abi::status::UNAVAILABLE)?;
        // SAFETY: host contract; `id` is readable for its length.
        match unsafe { register(id.as_ptr(), id.len(), Some(query)) } {
            abi::status::OK => Ok(()),
            rc => Err(rc),
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;

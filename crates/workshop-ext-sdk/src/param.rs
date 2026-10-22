//! Settings values that hook paths read lock-free: a [`Param`] is a
//! `static` holding one whole-number setting (an `AtomicU32` with its key,
//! default and inclusive range), plus an optional change callback.
//!
//! Menu writes go through [`Param::set`]: the value is clamped, stored and,
//! when it changed, the callback runs. Nothing touches the disk: where a
//! value persists is the unit's choice (for per-save settings see
//! [`crate::per_save::Params`]). Bind a param to a menu control with the
//! `param_*` methods of [`crate::settings::Builder`].
//!
//! ```
//! use workshop_ext_sdk::param::Param;
//! static ENABLED: Param = Param::flag("Enabled", false);
//! static HP: Param = Param::new("HpPercent", 150, 10, 2000);
//! assert!(!ENABLED.is_on());
//! assert!(HP.set(5000));
//! assert_eq!(HP.get(), 2000);
//! assert!(!HP.set(2000), "unchanged");
//! ```
use crate::packed::Field;
use core::sync::atomic::{AtomicU32, Ordering};

/// One whole-number setting.
#[derive(Debug)]
pub struct Param {
    /// Stable key (also the packed field key).
    pub key: &'static str,
    /// Value at process start and after a reset.
    pub default: u32,
    /// Smallest value.
    pub min: u32,
    /// Largest value.
    pub max: u32,
    on_change: Option<fn()>,
    value: AtomicU32,
}

impl Param {
    /// A setting over `min..=max` starting at `default` (checked at compile
    /// time in a `static`).
    pub const fn new(key: &'static str, default: u32, min: u32, max: u32) -> Param {
        assert!(
            min <= default && default <= max,
            "param: default outside its range"
        );
        Param {
            key,
            default,
            min,
            max,
            on_change: None,
            value: AtomicU32::new(default),
        }
    }

    /// A 0/1 flag.
    pub const fn flag(key: &'static str, default: bool) -> Param {
        Param::new(key, default as u32, 0, 1)
    }

    /// Run `f` after every change (menu write, reset or restore).
    pub const fn with_on_change(mut self, f: fn()) -> Param {
        self.on_change = Some(f);
        self
    }

    /// The current value.
    #[inline]
    pub fn get(&self) -> u32 {
        self.value.load(Ordering::Relaxed)
    }

    /// Whether the value is nonzero.
    #[inline]
    pub fn is_on(&self) -> bool {
        self.get() != 0
    }

    /// Clamp, store and, when the value changed, run the change callback.
    /// Returns whether it changed.
    pub fn set(&self, value: u32) -> bool {
        let v = value.clamp(self.min, self.max);
        if self.value.swap(v, Ordering::Relaxed) == v {
            return false;
        }
        if let Some(f) = self.on_change {
            f();
        }
        true
    }

    /// [`set`](Self::set) the default.
    pub fn reset(&self) -> bool {
        self.set(self.default)
    }

    /// The packed field of this param (same key, default and range).
    pub const fn field(&self) -> Field {
        Field::new(
            self.key,
            self.default as i64,
            self.min as i64,
            self.max as i64,
        )
    }

    /// A menu value as this param's whole number: NaN and infinity are
    /// ignored (`None`), negatives read as 0, the rest round half up.
    pub fn from_menu(value: f32) -> Option<u32> {
        if !value.is_finite() {
            return None;
        }
        Some(if value <= 0.0 {
            0
        } else {
            (value + 0.5) as u32
        })
    }
}

#[cfg(test)]
#[path = "param_tests.rs"]
mod tests;

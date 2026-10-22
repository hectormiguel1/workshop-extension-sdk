//! Byte-verified game sites with one shared policy.
//!
//! A unit lists every address it hooks, calls or depends on, with the exact
//! retail bytes read from the verified executable ([`sites!`](crate::sites!)), and checks
//! them all in `init` before touching anything ([`verify`]). Verifying a
//! site proves the executable; but other components hook game functions
//! too (Crux's own save adapter detours a game function with
//! MinHook before PHASE_LATE units run). So the policy depends on what the
//! unit does with a site ([`SiteKind`]):
//!
//! - a **`call`** site (the unit only calls the function) accepts the retail
//!   bytes or a recognised foreign entry detour: a 5-byte `jmp rel32` or a
//!   6-byte `jmp [abs32]` at the entry, every verified byte after it still
//!   retail, its destination outside the main image in committed executable
//!   memory. Calling through the detour is how hooks chain.
//! - a **`hook`** site (the unit writes its own hook or redirect there)
//!   accepts only the retail bytes; a foreign detour there is never
//!   overwritten ([`Problem::Occupied`]).
//! - an **`anchor`** (code or data the unit reads or patches by value)
//!   accepts only the retail bytes.
//!
//! Anything else is another build: verification fails closed.
use crate::image::Image;
use core::fmt;

/// What a unit does with a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteKind {
    /// Only called; a foreign entry detour is accepted.
    Call,
    /// Hooked or redirected by the unit; retail bytes only.
    Hook,
    /// Read or patched by value; retail bytes only.
    Anchor,
}

/// One verified site: a description, its RVA and the retail bytes there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    /// Function name or what the bytes are (used in log lines).
    pub name: &'static str,
    /// RVA in the main image.
    pub rva: u32,
    /// The retail bytes (at most [`MAX_SITE_BYTES`]).
    pub bytes: &'static [u8],
    /// What the unit does with it.
    pub kind: SiteKind,
}

/// Longest site [`verify`] compares.
pub const MAX_SITE_BYTES: usize = 512;

/// The shape of a foreign entry detour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    /// `E9 rel32` (MinHook and most x86 hook engines).
    Rel32,
    /// `FF 25 abs32`: `jmp dword ptr [abs32]`.
    Indirect,
}

impl Jump {
    /// Bytes of the jump instruction.
    pub const fn size(self) -> usize {
        match self {
            Jump::Rel32 => 5,
            Jump::Indirect => 6,
        }
    }

    /// `"jmp rel32"` or `"jmp [abs32]"`.
    pub const fn text(self) -> &'static str {
        match self {
            Jump::Rel32 => "jmp rel32",
            Jump::Indirect => "jmp [abs32]",
        }
    }
}

/// What was found at a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Exactly the retail bytes.
    Retail,
    /// Another component's entry detour over retail code.
    Foreign {
        /// The jump at the entry.
        jump: Jump,
        /// Its destination.
        dest: usize,
    },
    /// Neither: another build (or a patch the unit does not know).
    Mismatch,
}

/// The process memory a verdict may consult (the live process, or a fake).
pub trait Memory {
    /// A readable dword at `at`, or `None`.
    fn read_u32(&self, at: usize) -> Option<u32>;
    /// Whether `at` is committed executable memory.
    fn executable(&self, at: usize) -> bool;
}

/// A [`Memory`] with no foreign code: every detour is a mismatch. The
/// strict policy for portable tests and for images not mapped in-process.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoForeignCode;

impl Memory for NoForeignCode {
    fn read_u32(&self, _: usize) -> Option<u32> {
        None
    }
    fn executable(&self, _: usize) -> bool {
        false
    }
}

/// The bytes `found` at `entry` (the site's absolute address) against the
/// retail `expected`, for the main `image`.
pub fn classify(
    entry: usize,
    expected: &[u8],
    found: &[u8],
    image: Image,
    memory: &dyn Memory,
) -> Verdict {
    if found == expected {
        return Verdict::Retail;
    }
    let jump = match found {
        [0xE9, ..] => Jump::Rel32,
        [0xFF, 0x25, ..] => Jump::Indirect,
        _ => return Verdict::Mismatch,
    };
    let n = jump.size();
    // At least one verified byte past the jump must still be retail.
    if found.len() != expected.len() || expected.len() <= n || found[n..] != expected[n..] {
        return Verdict::Mismatch;
    }
    let operand = u32::from_le_bytes([found[n - 4], found[n - 3], found[n - 2], found[n - 1]]);
    let dest = match jump {
        Jump::Rel32 => (entry as u32).wrapping_add(5).wrapping_add(operand),
        Jump::Indirect => match memory.read_u32(operand as usize) {
            Some(dest) => dest,
            None => return Verdict::Mismatch,
        },
    } as usize;
    if dest == 0 || image.holds(dest) || !memory.executable(dest) {
        return Verdict::Mismatch;
    }
    Verdict::Foreign { jump, dest }
}

/// Whether a unit may use a site of `kind` with this verdict.
pub const fn admits(kind: SiteKind, verdict: Verdict) -> bool {
    match verdict {
        Verdict::Retail => true,
        Verdict::Foreign { .. } => matches!(kind, SiteKind::Call),
        Verdict::Mismatch => false,
    }
}

/// Why a site fails verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The site lies outside the loaded image (or could not be read).
    OutsideImage,
    /// The loaded bytes differ (another build).
    Bytes,
    /// A `hook`/`anchor` site already carries another component's detour;
    /// the unit never overwrites it.
    Occupied {
        /// The jump at the entry.
        jump: Jump,
        /// Its destination.
        dest: usize,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::OutsideImage => f.write_str("outside the loaded image"),
            Problem::Bytes => f.write_str("bytes differ from the verified build"),
            Problem::Occupied { jump, dest } => write!(
                f,
                "already hooked by another component ({} -> {dest:#010x}); not overwritten",
                jump.text()
            ),
        }
    }
}

/// A reported site: a problem, or an accepted foreign detour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finding {
    /// The site fails verification.
    Problem(Problem),
    /// A `call` site whose foreign entry detour was accepted.
    Foreign {
        /// The jump at the entry.
        jump: Jump,
        /// Its destination.
        dest: usize,
    },
}

/// Result of [`verify`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Sites that fail (the unit must not install).
    pub failed: usize,
    /// Sites with any foreign mismatch-free detour (accepted or occupied).
    pub foreign: usize,
}

impl Summary {
    /// Whether every site was admitted.
    pub const fn passed(&self) -> bool {
        self.failed == 0
    }
}

/// Check one site: `read(rva, buffer)` fills `buffer` from the loaded image
/// and returns `false` when the range cannot be read.
pub fn check(
    site: &Site,
    image: Image,
    read: &dyn Fn(u32, &mut [u8]) -> bool,
    memory: &dyn Memory,
) -> Result<Verdict, Problem> {
    let mut buffer = [0u8; MAX_SITE_BYTES];
    let len = site.bytes.len();
    let Some(window) = buffer.get_mut(..len) else {
        return Err(Problem::OutsideImage);
    };
    if !image.contains(site.rva, len) || !read(site.rva, window) {
        return Err(Problem::OutsideImage);
    }
    match classify(image.address(site.rva), site.bytes, window, image, memory) {
        Verdict::Mismatch => Err(Problem::Bytes),
        Verdict::Foreign { jump, dest } if !admits(site.kind, Verdict::Foreign { jump, dest }) => {
            Err(Problem::Occupied { jump, dest })
        }
        verdict => Ok(verdict),
    }
}

/// Check every site, reporting each problem and each accepted foreign
/// detour to `report` (retail sites are not reported).
pub fn verify(
    sites: &[Site],
    image: Image,
    read: &dyn Fn(u32, &mut [u8]) -> bool,
    memory: &dyn Memory,
    report: &mut dyn FnMut(&Site, Finding),
) -> Summary {
    let mut out = Summary::default();
    for site in sites {
        match check(site, image, read, memory) {
            Ok(Verdict::Foreign { jump, dest }) => {
                out.foreign += 1;
                report(site, Finding::Foreign { jump, dest });
            }
            Ok(_) => {}
            Err(problem) => {
                out.failed += 1;
                out.foreign += usize::from(matches!(problem, Problem::Occupied { .. }));
                report(site, Finding::Problem(problem));
            }
        }
    }
    out
}

/// Displays a byte slice as `0f b6 45`.
pub fn hex_bytes(bytes: &[u8]) -> HexBytes<'_> {
    HexBytes(bytes)
}

/// See [`hex_bytes`].
pub struct HexBytes<'a>(pub &'a [u8]);

impl fmt::Display for HexBytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Compile-time checks of a site table: nonempty, at most
/// [`MAX_SITE_BYTES`] bytes each. Used by [`sites!`](crate::sites!).
pub const fn check_table(sites: &[Site]) -> bool {
    let mut i = 0;
    while i < sites.len() {
        if sites[i].bytes.is_empty() || sites[i].bytes.len() > MAX_SITE_BYTES {
            return false;
        }
        i += 1;
    }
    true
}

/// Declares a table of verified sites, one `"name" @ rva = bytes, kind;`
/// per site:
///
/// * `rva`: a literal, a named constant or a parenthesised expression;
/// * `bytes`: an array literal, a byte-string literal, the name of an
///   array or slice constant, or `{ expr }` of any type that coerces to
///   `&[u8]` (such as a `&[u8; N]` constant);
/// * `kind`: `call`, `hook` or `anchor` ([`SiteKind`]).
///
/// A compile-time check rejects empty sites and sites longer than
/// [`MAX_SITE_BYTES`].
///
/// ```
/// use workshop_ext_sdk::sites::{Site, SiteKind};
/// pub const SET_HP: u32 = 0x0006_e3f0;
/// const HEAD: [u8; 3] = [0x55, 0x8b, 0xec];
/// workshop_ext_sdk::sites! {
///     /// Every site the unit uses.
///     pub const SITES = {
///         "BattleChara_SetHp" @ SET_HP = [0x55, 0x8b, 0xec, 0x83, 0xec, 0x18], call;
///         "Btl_SetupBattleStart" @ 0x000d_2400 = b"\x55\x8b\xec\x81\xec\xe4\x07\x00\x00", hook;
///         "750.0f" @ 0x00cc_e9dc = [0x00, 0x80, 0x3b, 0x44], anchor;
///         "a named array" @ 0x10 = HEAD, anchor;
///         "any &[u8] expression" @ 0x20 = { HEAD.split_at(2).0 }, anchor;
///     }
/// }
/// assert_eq!(SITES.len(), 5);
/// assert_eq!(SITES[4].bytes, [0x55, 0x8b]);
/// assert_eq!(SITES[0].kind, SiteKind::Call);
/// ```
#[macro_export]
macro_rules! sites {
    (
        $(#[$meta:meta])*
        $vis:vis const $name:ident = {
            $( $site:literal @ $rva:tt = $bytes:tt $(,)? $kind:ident; )*
        }
    ) => {
        $(#[$meta])*
        $vis const $name: &[$crate::sites::Site] = &[
            $( $crate::sites::Site {
                name: $site,
                rva: $rva,
                bytes: $crate::__site_bytes!($bytes),
                kind: $crate::__site_kind!($kind),
            }, )*
        ];
        const _: () = assert!(
            $crate::sites::check_table($name),
            "sites!: every site needs 1..=512 bytes"
        );
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __site_bytes {
    ([$($b:expr),* $(,)?]) => { &[$($b),*] };
    ($bytes:literal) => { $bytes };
    ($bytes:ident) => {{
        const BYTES: &[u8] = &$bytes;
        BYTES
    }};
    ({ $bytes:expr }) => {{
        const BYTES: &[u8] = $bytes;
        BYTES
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __site_kind {
    (call) => {
        $crate::sites::SiteKind::Call
    };
    (hook) => {
        $crate::sites::SiteKind::Hook
    };
    (anchor) => {
        $crate::sites::SiteKind::Anchor
    };
}

#[cfg(test)]
#[path = "sites_tests.rs"]
mod tests;

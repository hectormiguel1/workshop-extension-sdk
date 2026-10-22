//! A unit's own menu texts for the FINAL FANTASY XIII Mod Settings menu.
//!
//! The XIII menu adapter shows no literal: it looks every string of a
//! provider up in the game's ZTR resources under `$ms` + the low 48 bits of
//! the FNV-1a 64 hash of the English source ([`key`]), in the locale the
//! game runs in, and shows "Unavailable" for a missing entry. A unit
//! therefore ships its strings in its own mod container, in all eleven
//! system locales ([`LOCALES`]).
//!
//! [`required`] lists what the adapter will look up for a provider (plus
//! texts the unit itself looks up at run time); [`problems`] checks a
//! catalog against it and [`missing_in`] checks a built container's text
//! dump.
//!
//! ```
//! use workshop_ext_sdk::texts;
//! assert_eq!(texts::key("Gil"), "$ms8b1998869903");
//! let catalog: &[texts::Entry] = &[("Gil", ["Gil", "Gil", "Gils", "Gil", "Guil", "Guiles", "ギル", "Гили", "金幣", "金币", "길"])];
//! assert!(texts::problems(&["Gil"], catalog).is_empty());
//! ```
use crate::abi::SettingsProvider;
use std::collections::BTreeSet;

/// The eleven system locales, in catalog order.
pub const LOCALES: [&str; 11] = [
    "us", "uk", "fr", "gr", "it", "sp", "jp", "ru", "ch", "ck", "kr",
];

/// One catalog entry: the English source and its text per [`LOCALES`].
pub type Entry = (&'static str, [&'static str; 11]);

/// The ZTR key of `source`: `$ms` + the low 48 bits of FNV-1a 64, as 12
/// lowercase hex digits.
pub fn key(source: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in source.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("$ms{:012x}", hash & 0xffff_ffff_ffff)
}

/// The `{N}` placeholders of `text`, sorted (with repeats).
pub fn placeholders(text: &str) -> Vec<u8> {
    let mut out: Vec<u8> = text
        .as_bytes()
        .windows(3)
        .filter(|w| w[0] == b'{' && w[1].is_ascii_digit() && w[2] == b'}')
        .map(|w| w[1])
        .collect();
    out.sort_unstable();
    out
}

/// What the XIII adapter looks up for `provider`, then `runtime`: the
/// provider name without a leading `FF13 ` (the adapter's short name) and
/// every [`crate::settings::menu_strings`] entry, without repeats.
///
/// # Safety
/// `provider` comes from [`crate::settings::Builder::finish`].
pub unsafe fn required(provider: &SettingsProvider, runtime: &[&'static str]) -> Vec<&'static str> {
    // SAFETY: caller contract.
    let menu = unsafe { crate::settings::menu_strings(provider) };
    let mut out: Vec<&'static str> = Vec::new();
    for s in menu.into_iter().chain(runtime.iter().copied()) {
        let s = s.strip_prefix("FF13 ").unwrap_or(s);
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Every `required` source missing from `catalog`, and every entry of a
/// required source with an empty locale or other placeholders than its
/// source. Empty when the catalog is complete.
pub fn problems(required: &[&str], catalog: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    for s in required {
        match catalog.iter().find(|(source, _)| source == s) {
            None => out.push(format!("missing: {s:?}")),
            Some((_, texts)) => {
                for (locale, t) in LOCALES.iter().zip(texts) {
                    if t.is_empty() {
                        out.push(format!("{s:?}: empty {locale}"));
                    } else if placeholders(t) != placeholders(s) {
                        out.push(format!("{s:?}: {locale} placeholders differ"));
                    }
                }
            }
        }
    }
    out
}

/// The `key locale (source)` of every required text a container lacks.
/// `rows` is a text dump, one `key<TAB>locale<TAB>text` row per line (the
/// shape of CrystalWorkshop's `query` output for `SELECT key, locale, text FROM
/// texts`); an empty text counts as missing, other lines are ignored.
pub fn missing_in(required: &[&str], rows: &str) -> Vec<String> {
    let have: BTreeSet<(&str, &str)> = rows
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            let (k, locale, text) = (f.next()?, f.next()?, f.next()?);
            (!text.trim().is_empty()).then_some((k, locale))
        })
        .collect();
    let mut out = Vec::new();
    for s in required {
        let k = key(s);
        for locale in LOCALES {
            if !have.contains(&(k.as_str(), locale)) {
                out.push(format!("{k} {locale} ({s:?})"));
            }
        }
    }
    out
}

/// The catalog as JSON lines, one `{"key","source","texts":{locale:text}}`
/// object per required entry (for container build tools).
pub fn json_lines(required: &[&str], catalog: &[Entry]) -> String {
    fn quote(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
    let mut out = String::new();
    for s in required {
        let Some((_, texts)) = catalog.iter().find(|(source, _)| source == s) else {
            continue;
        };
        let texts: Vec<String> = LOCALES
            .iter()
            .zip(texts)
            .map(|(l, t)| format!("{}:{}", quote(l), quote(t)))
            .collect();
        out.push_str(&format!(
            "{{\"key\":{},\"source\":{},\"texts\":{{{}}}}}\n",
            quote(&key(s)),
            quote(s),
            texts.join(",")
        ));
    }
    out
}

#[cfg(test)]
#[path = "texts_tests.rs"]
mod tests;

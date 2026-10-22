//! The golden vectors were produced by a released warp unit's own
//! hand-written record encoder, so the derive is byte-for-byte the packed
//! format units already store.
use super::*;
use crate::PackedRecord;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const DEFAULTS: &str = "0101000000be13000000000000000000";
const FULL: &str = "012bc031c0d229e9ffffff7f0d000020000c080052657475726e4d61705365747a3130355f61726b00090a0052657475726e506f73312e35202d3220333030030504004f746865726b657074";
const LEGACY: &str = "010a0000007c27803401000000000000";

/// The warp unit's host-region schema 2 options.
#[derive(Clone, Debug, PartialEq, PackedRecord)]
#[packed(version = 1, schema = 2, migrate = from_legacy, valid = origin_marker)]
struct Options {
    #[packed(default = 1, max = 11)]
    destination: u8,
    #[packed(max = 2)]
    debug_page: u8,
    #[packed(key = "DebugPointPage1", max = 200)]
    debug_point_1: u8,
    #[packed(key = "DebugPointPage2", max = 200)]
    debug_point_2: u8,
    #[packed(key = "DebugPointPage3", max = 200)]
    debug_point_3: u8,
    #[packed(default = 20217, min = 1, max = 29999)]
    battle_id: u16,
    verbose_log: bool,
    #[packed(key = "EnableZone107Probe")]
    probe: bool,
    #[packed(key = "SkipArkEntryEvent")]
    skip_ark_entry: bool,
    #[packed(max = 9)]
    ark_activation: u8,
    ark_boss_story_callback: bool,
    #[packed(max = u32::MAX)]
    return_zone: u32,
    #[packed(key = "Probe107OriginZone", max = u32::MAX)]
    probe_zone: u32,
    #[packed(key = "Probe107OriginValid")]
    probe_valid: bool,
    #[packed(text)]
    text: Vec<TextEntry>,
}

/// Schema 1 / legacy DAT: the same with a leading `Enabled` field.
const LEGACY_LAYOUT: Layout = Layout {
    version: 1,
    schema: 1,
    fields: &[
        Field::new("Enabled", 1, 0, 1),
        Field::new("Destination", 1, 0, 11),
        Field::new("DebugPage", 0, 0, 2),
        Field::new("DebugPointPage1", 0, 0, 200),
        Field::new("DebugPointPage2", 0, 0, 200),
        Field::new("DebugPointPage3", 0, 0, 200),
        Field::new("BattleId", 20217, 1, 29999),
        Field::new("VerboseLog", 0, 0, 1),
        Field::new("EnableZone107Probe", 0, 0, 1),
        Field::new("SkipArkEntryEvent", 0, 0, 1),
        Field::new("ArkActivation", 0, 0, 9),
        Field::new("ArkBossStoryCallback", 0, 0, 1),
        Field::new("ReturnZone", 0, 0, 4_294_967_295),
        Field::new("Probe107OriginZone", 0, 0, 4_294_967_295),
        Field::new("Probe107OriginValid", 0, 0, 1),
    ],
};

fn from_legacy(schema: u32, bytes: &[u8]) -> Option<Options> {
    if schema != 1 {
        return None;
    }
    let (values, mut text) = decode(&LEGACY_LAYOUT, bytes)?;
    text.retain(|e| !(e.section == 0 && e.key.eq_ignore_ascii_case("Enabled")));
    Some(Options::from_parts(&values[1..], text))
}

fn origin_marker(o: &Options) -> bool {
    !o.probe_valid || o.probe_zone != 0
}

#[test]
fn the_derived_layout_is_the_warp_units() {
    let keys: Vec<&str> = Options::LAYOUT.fields.iter().map(|f| f.key).collect();
    let legacy: Vec<&str> = LEGACY_LAYOUT.fields[1..].iter().map(|f| f.key).collect();
    assert_eq!(keys, legacy);
    assert_eq!(Options::LAYOUT.fields, &LEGACY_LAYOUT.fields[1..]);
    assert_eq!((Options::LAYOUT.version, Options::LAYOUT.schema), (1, 2));
    assert_eq!(Options::LAYOUT.packed_len(), 16);
    assert!(Options::LAYOUT.is_valid() && LEGACY_LAYOUT.is_valid());
    assert_eq!(Options::LAYOUT.field("returnzone"), Some(11));
    assert_eq!(Field::new("x", 0, 0, 0).bits(), 0);
    assert_eq!(Field::new("x", 0, 1, 29999).bits(), 15);
}

#[test]
fn records_match_the_golden_bytes() {
    let defaults = Options::defaults();
    assert_eq!(defaults.encode().unwrap(), hex(DEFAULTS));
    assert_eq!(Options::decode(&hex(DEFAULTS)), Some(defaults));
    let full = Options {
        destination: 11,
        debug_page: 2,
        debug_point_2: 199,
        battle_id: 10060,
        verbose_log: true,
        ark_activation: 9,
        return_zone: u32::MAX,
        probe_zone: 107,
        probe_valid: true,
        text: vec![
            TextEntry::setting("ReturnMapSet", "z105_ark"),
            TextEntry::setting("ReturnPos", "1.5 -2 300"),
            TextEntry {
                section: 3,
                key: "Other".into(),
                value: "kept".into(),
            },
        ],
        ..Options::defaults()
    };
    assert_eq!(full.encode().unwrap(), hex(FULL));
    let back = Options::decode(&hex(FULL)).unwrap();
    assert_eq!(back, full);
    assert_eq!(back.setting_text("returnmapset"), Some("z105_ark"));
    assert_eq!(Options::load(2, &hex(FULL)), Some(full));
    // Schema 1 migrates (dropping Enabled).
    let legacy = Options::load(1, &hex(LEGACY)).unwrap();
    assert_eq!((legacy.destination, legacy.return_zone), (5, 1234));
    assert_eq!(Options::load(9, &hex(LEGACY)), None);
}

#[test]
fn malformed_records_are_refused() {
    let layout = Options::LAYOUT;
    let good = hex(FULL);
    // Version byte, short, oversized, value above max, padding bits.
    let mut v = good.clone();
    v[0] = 2;
    assert_eq!(decode(&layout, &v), None);
    assert_eq!(decode(&layout, &good[..10]), None);
    assert_eq!(decode(&layout, &vec![1u8; LIMIT + 1]), None);
    let mut over = hex(DEFAULTS);
    over[1] = 0x0f; // destination 15 > 11
    assert_eq!(decode(&layout, &over), None);
    let mut pad = hex(DEFAULTS);
    pad[15] = 0x80;
    assert_eq!(decode(&layout, &pad), None);
    // Tail: truncated header/key/value, bad section, empty key, long value,
    // a section 0 key naming a field, duplicates, NUL, too many entries.
    let base = hex(DEFAULTS);
    let tail = |t: &[u8]| [base.as_slice(), t].concat();
    for t in [
        &b"\x00\x01"[..],
        b"\x00\x03\x00\x00ab",
        b"\x00\x01\x05\x00ab",
        b"\x05\x01\x00\x00a",
        b"\x00\x00\x00\x00",
        b"\x00\x01\x01\x10a",
        b"\x00\x08\x00\x00Battleid",
        b"\x00\x01\x00\x00a\x00\x01\x00\x00A",
        b"\x00\x01\x01\x00a\x00",
        b"\x00\x01\x00\x00\xff",
        b"\x00\x01\x01\x00a\xff",
    ] {
        assert_eq!(decode(&layout, &tail(t)), None, "{t:x?}");
    }
    let mut many = base.clone();
    for i in 0..=MAX_TEXT {
        let key = format!("k{i}");
        many.extend([1, key.len() as u8, 0, 0]);
        many.extend(key.as_bytes());
    }
    assert_eq!(decode(&layout, &many), None);
    // Encoder refusals.
    let values = layout.defaults();
    assert_eq!(encode(&layout, &values[1..], &[]), None);
    let mut bad = values.clone();
    bad[0] = 12;
    assert_eq!(encode(&layout, &bad, &[]), None);
    for entry in [
        TextEntry::setting("", "x"),
        TextEntry::setting("destination", "x"),
        TextEntry::setting("a\0", "x"),
        TextEntry::setting("a", &"v".repeat(MAX_VALUE + 1)),
        TextEntry {
            section: 5,
            ..TextEntry::setting("a", "")
        },
    ] {
        assert_eq!(encode(&layout, &values, &[entry]), None);
    }
    let dup = [TextEntry::setting("a", ""), TextEntry::setting("A", "")];
    assert_eq!(encode(&layout, &values, &dup), None);
    let big: Vec<TextEntry> = (0..17)
        .map(|i| TextEntry::setting(&format!("k{i}"), &"v".repeat(4000)))
        .collect();
    assert_eq!(encode(&layout, &values, &big), None);
    let too_many: Vec<TextEntry> = (0..=MAX_TEXT)
        .map(|i| TextEntry::setting(&format!("k{i}"), ""))
        .collect();
    assert_eq!(encode(&layout, &values, &too_many), None);
    // An invalid record (marker without zone) neither encodes nor decodes.
    let invalid = Options {
        probe_valid: true,
        probe_zone: 0,
        ..Options::defaults()
    };
    assert_eq!(invalid.encode(), None);
    assert_eq!(
        Options::decode(&encode(&layout, &invalid.values(), &[]).unwrap()),
        None
    );
}

/// Without text, migrate or valid: the provided defaults apply.
#[derive(Debug, PartialEq, PackedRecord)]
struct Minimal {
    on: bool,
    #[packed(min = -3, max = 3)]
    delta: i8,
}

#[test]
fn trait_defaults_apply_to_minimal_records() {
    let m = Minimal {
        on: true,
        delta: -3,
    };
    let bytes = m.encode().unwrap();
    assert_eq!(bytes, [1, 0b0000_0001]);
    assert_eq!(
        Minimal::decode(&bytes),
        Some(Minimal {
            on: true,
            delta: -3
        })
    );
    assert!(m.text().is_empty());
    assert_eq!(Minimal::load(1, &bytes), Minimal::decode(&bytes));
    assert_eq!(Minimal::load(7, &bytes), None);
    assert_eq!(
        Minimal::from_parts(&[], Vec::new()),
        Minimal {
            on: false,
            delta: -3
        }
    );
    const DUPLICATE: Layout = Layout {
        fields: &[Field::new("a", 0, 0, 1), Field::new("A", 0, 0, 1)],
        ..Minimal::LAYOUT
    };
    const DEFAULT_OUT: Layout = Layout {
        fields: &[Field::new("a", 5, 0, 1)],
        ..Minimal::LAYOUT
    };
    const RANGE_OVERFLOW: Layout = Layout {
        fields: &[Field::new("a", 0, i64::MIN, i64::MAX)],
        ..Minimal::LAYOUT
    };
    const INVERTED: Layout = Layout {
        fields: &[Field::new("a", 0, 1, 0)],
        ..Minimal::LAYOUT
    };
    for layout in [DUPLICATE, DEFAULT_OUT, RANGE_OVERFLOW, INVERTED] {
        assert!(!layout.is_valid());
    }
    for v in [
        u8::MAX.to_packed(),
        u16::MAX.to_packed(),
        i16::MIN.to_packed(),
        i32::MIN.to_packed(),
        5i64.to_packed(),
    ] {
        assert_ne!(v, 0);
    }
    assert_eq!(
        (
            u16::from_packed(9),
            i16::from_packed(-9),
            i32::from_packed(-1),
            i64::from_packed(4)
        ),
        (9, -9, -1, 4)
    );
}

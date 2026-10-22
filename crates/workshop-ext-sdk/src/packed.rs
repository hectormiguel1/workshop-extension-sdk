//! Bit-packed storage records: the compact, versioned format units use for
//! their host storage regions.
//!
//! Wire format:
//!
//! * byte 0: the format version ([`Layout::version`]);
//! * the numeric fields, LSB-first in their frozen declaration order, each
//!   storing `value - min` in `ceil(log2(max - min + 1))` bits; unused high
//!   bits of the last byte are zero;
//! * an optional text tail of entries: section `u8` (0 = Settings, < 5),
//!   key length `u8`, value length `u16` LE, key, value (UTF-8, no NUL;
//!   a section 0 key never names a numeric field; keys are unique per
//!   section, case-insensitively).
//!
//! A record is at most [`LIMIT`] bytes. Field order, ranges and the version
//! are frozen once released: add fields with a new [`Layout::schema`] and
//! [`PackedRecord::migrate`] old bytes. Derive the trait with
//! `#[derive(PackedRecord)]` (feature `derive`), or implement it by hand.

/// Largest accepted record (bytes).
pub const LIMIT: usize = 65536;
/// Most text entries per record.
pub const MAX_TEXT: usize = 512;
/// Longest text key.
pub const MAX_KEY: usize = 255;
/// Longest text value.
pub const MAX_VALUE: usize = 4096;
/// Text sections are `0..SECTIONS`.
pub const SECTIONS: u8 = 5;

/// One numeric field: its frozen key, default and inclusive range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// Stable key (PascalCase by convention; matched case-insensitively).
    pub key: &'static str,
    /// Value of a fresh record.
    pub default: i64,
    /// Smallest value.
    pub min: i64,
    /// Largest value.
    pub max: i64,
}

impl Field {
    /// A field (const constructor).
    pub const fn new(key: &'static str, default: i64, min: i64, max: i64) -> Self {
        Self {
            key,
            default,
            min,
            max,
        }
    }

    /// Bits the field occupies: `ceil(log2(max - min + 1))`.
    pub const fn bits(self) -> usize {
        (64 - (self.max - self.min).leading_zeros()) as usize
    }

    /// Whether `value` is in range.
    pub const fn admits(self, value: i64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// A record layout: format version byte, storage schema and fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Byte 0 of every record.
    pub version: u8,
    /// The host storage schema this layout is written with.
    pub schema: u32,
    /// The numeric fields, in wire order.
    pub fields: &'static [Field],
}

impl Layout {
    /// Bytes of the version byte plus the numeric fields.
    pub const fn packed_len(&self) -> usize {
        packed_len(self.fields)
    }

    /// Whether every field is well formed (`min <= default <= max`, range
    /// below 2^63) and keys are unique, case-insensitively.
    pub const fn is_valid(&self) -> bool {
        let f = self.fields;
        let mut i = 0;
        while i < f.len() {
            if f[i].min > f[i].max
                || !f[i].admits(f[i].default)
                || f[i].max.checked_sub(f[i].min).is_none()
            {
                return false;
            }
            let mut j = i + 1;
            while j < f.len() {
                if f[i].key.eq_ignore_ascii_case(f[j].key) {
                    return false;
                }
                j += 1;
            }
            i += 1;
        }
        true
    }

    /// The index of `key` (case-insensitive).
    pub fn field(&self, key: &str) -> Option<usize> {
        field(self.fields, key)
    }

    /// The default values.
    pub fn defaults(&self) -> Vec<i64> {
        self.fields.iter().map(|f| f.default).collect()
    }
}

/// Bytes of the version byte plus `fields`.
pub const fn packed_len(fields: &[Field]) -> usize {
    let mut bits = 0;
    let mut i = 0;
    while i < fields.len() {
        bits += fields[i].bits();
        i += 1;
    }
    1 + bits.div_ceil(8)
}

/// The index of `key` in `fields` (case-insensitive).
pub fn field(fields: &[Field], key: &str) -> Option<usize> {
    fields.iter().position(|f| f.key.eq_ignore_ascii_case(key))
}

/// One text tail entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEntry {
    /// Section, `0..SECTIONS` (0 = Settings).
    pub section: u8,
    /// Key (1..=255 bytes).
    pub key: String,
    /// Value (≤ 4096 bytes).
    pub value: String,
}

impl TextEntry {
    /// A section 0 (Settings) entry.
    pub fn setting(key: &str, value: &str) -> Self {
        Self {
            section: 0,
            key: key.into(),
            value: value.into(),
        }
    }
}

fn text_ok(fields: &[Field], text: &[TextEntry], entry: &TextEntry) -> bool {
    entry.section < SECTIONS
        && !entry.key.is_empty()
        && entry.key.len() <= MAX_KEY
        && entry.value.len() <= MAX_VALUE
        && !entry.key.contains('\0')
        && !entry.value.contains('\0')
        && !(entry.section == 0 && field(fields, &entry.key).is_some())
        && !text
            .iter()
            .any(|e| e.section == entry.section && e.key.eq_ignore_ascii_case(&entry.key))
}

/// Pack `values` (one per field) and the text tail; `None` for a value out
/// of range, an invalid or duplicate text entry, or a record above [`LIMIT`].
pub fn encode(layout: &Layout, values: &[i64], text: &[TextEntry]) -> Option<Vec<u8>> {
    let fields = layout.fields;
    if values.len() != fields.len() || text.len() > MAX_TEXT {
        return None;
    }
    let mut data = vec![0u8; packed_len(fields)];
    data[0] = layout.version;
    let mut bit = 8;
    for (f, value) in fields.iter().zip(values) {
        if !f.admits(*value) {
            return None;
        }
        let raw = (value - f.min) as u64;
        for i in 0..f.bits() {
            data[bit / 8] |= (((raw >> i) & 1) as u8) << (bit % 8);
            bit += 1;
        }
    }
    for (index, entry) in text.iter().enumerate() {
        if !text_ok(fields, &text[..index], entry)
            || data.len() + 4 + entry.key.len() + entry.value.len() > LIMIT
        {
            return None;
        }
        data.extend([entry.section, entry.key.len() as u8]);
        data.extend((entry.value.len() as u16).to_le_bytes());
        data.extend(entry.key.as_bytes());
        data.extend(entry.value.as_bytes());
    }
    Some(data)
}

/// Unpack a record of `layout`: the values (one per field) and the text
/// tail; `None` for another version, a short/oversized record, a value out
/// of range, nonzero padding bits or a malformed tail.
pub fn decode(layout: &Layout, data: &[u8]) -> Option<(Vec<i64>, Vec<TextEntry>)> {
    let fields = layout.fields;
    if data.len() < packed_len(fields) || data.len() > LIMIT || data[0] != layout.version {
        return None;
    }
    let mut values = layout.defaults();
    let mut bit = 8;
    for (f, value) in fields.iter().zip(&mut values) {
        let mut raw = 0u64;
        for i in 0..f.bits() {
            raw |= (((data[bit / 8] >> (bit % 8)) & 1) as u64) << i;
            bit += 1;
        }
        *value = f.min + raw as i64;
        if *value > f.max {
            return None;
        }
    }
    if bit % 8 != 0 && data[bit / 8] >> (bit % 8) != 0 {
        return None;
    }
    let mut text: Vec<TextEntry> = Vec::new();
    let mut at = packed_len(fields);
    while at < data.len() {
        let header = data.get(at..at + 4)?;
        let section = header[0];
        let key_len = header[1] as usize;
        let len = u16::from_le_bytes([header[2], header[3]]) as usize;
        if section >= SECTIONS || key_len == 0 || len > MAX_VALUE || text.len() >= MAX_TEXT {
            return None;
        }
        at += 4;
        let key = core::str::from_utf8(data.get(at..at + key_len)?)
            .ok()?
            .to_owned();
        at += key_len;
        let value = core::str::from_utf8(data.get(at..at + len)?)
            .ok()?
            .to_owned();
        at += len;
        let entry = TextEntry {
            section,
            key,
            value,
        };
        if !text_ok(fields, &text, &entry) {
            return None;
        }
        text.push(entry);
    }
    Some((values, text))
}

/// A numeric field type of a [`PackedRecord`].
pub trait PackedValue: Copy {
    /// The value as stored.
    fn to_packed(self) -> i64;
    /// The value of a decoded, range-checked field.
    fn from_packed(value: i64) -> Self;
}

impl PackedValue for bool {
    fn to_packed(self) -> i64 {
        i64::from(self)
    }
    fn from_packed(value: i64) -> Self {
        value != 0
    }
}

macro_rules! packed_int {
    ($($t:ty),*) => {$(
        impl PackedValue for $t {
            fn to_packed(self) -> i64 {
                self as i64
            }
            fn from_packed(value: i64) -> Self {
                value as $t
            }
        }
    )*};
}
packed_int!(u8, u16, u32, i8, i16, i32, i64);

/// A typed record over a frozen [`Layout`]. `#[derive(PackedRecord)]`
/// implements the required items; the provided methods encode, decode,
/// migrate and validate.
pub trait PackedRecord: Sized {
    /// The frozen layout.
    const LAYOUT: Layout;

    /// The numeric fields, in layout order.
    fn values(&self) -> Vec<i64>;
    /// The text tail (empty for records without one).
    fn text(&self) -> &[TextEntry];
    /// A record from decoded, range-checked values and a text tail.
    fn from_parts(values: &[i64], text: Vec<TextEntry>) -> Self;

    /// Bytes of another (older) schema as a current record; `None` by
    /// default (unknown schemas are refused).
    fn migrate(schema: u32, bytes: &[u8]) -> Option<Self> {
        let _ = (schema, bytes);
        None
    }

    /// Cross-field validity beyond the per-field ranges (default: valid).
    fn is_valid(&self) -> bool {
        true
    }

    /// A record of the defaults and no text.
    fn defaults() -> Self {
        Self::from_parts(&Self::LAYOUT.defaults(), Vec::new())
    }

    /// The packed bytes; `None` when out of range or not [`is_valid`](Self::is_valid).
    fn encode(&self) -> Option<Vec<u8>> {
        if !self.is_valid() {
            return None;
        }
        encode(&Self::LAYOUT, &self.values(), self.text())
    }

    /// A record of the current layout; `None` when malformed or invalid.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let (values, text) = decode(&Self::LAYOUT, bytes)?;
        Some(Self::from_parts(&values, text)).filter(Self::is_valid)
    }

    /// Stored bytes of `schema`: the current schema decodes, any other
    /// goes through [`migrate`](Self::migrate). Either way the result must
    /// be valid.
    fn load(schema: u32, bytes: &[u8]) -> Option<Self> {
        if schema == Self::LAYOUT.schema {
            Self::decode(bytes)
        } else {
            Self::migrate(schema, bytes).filter(Self::is_valid)
        }
    }

    /// The value of a text entry of section 0 (empty reads as absent).
    fn setting_text(&self, key: &str) -> Option<&str> {
        self.text()
            .iter()
            .find(|e| e.section == 0 && e.key.eq_ignore_ascii_case(key))
            .map(|e| e.value.as_str())
            .filter(|v| !v.is_empty())
    }
}

#[cfg(test)]
#[path = "packed_tests.rs"]
mod tests;

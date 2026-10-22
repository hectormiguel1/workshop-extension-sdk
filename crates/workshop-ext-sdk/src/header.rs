//! Test support: a small parser for the host's C headers (struct fields,
//! `#define` constants, `_Static_assert`s) so the Rust mirrors in
//! [`crate::abi`] are compared against the contract, not against a copy.
use std::collections::BTreeMap;

/// `ff13_host.h`.
pub const HOST: &str = include_str!("../include/ff13_host.h");
/// `ff13_settings.h`.
pub const SETTINGS: &str = include_str!("../include/ff13_settings.h");
/// `ff13_save_events.h`.
pub const SAVE_EVENTS: &str = include_str!("../include/ff13_save_events.h");
/// The host's Rust settings contract (text extension).
pub const SETTINGS_CONTRACT: &str = include_str!("../include/settings_contract.rs");

/// The three headers, settings and save events first (the host header
/// includes them).
pub fn text() -> String {
    [SETTINGS, SAVE_EVENTS, HOST].join("\n")
}

fn strip_comments(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        rest = rest[at..]
            .find("*/")
            .map_or("", |end| &rest[at + end + 2..]);
    }
    out.push_str(rest);
    out.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One field's storage class.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Scalar(usize),
    Pointer,
    Nested(String),
}

/// A parsed `typedef struct`.
#[derive(Clone, Debug)]
pub struct Struct {
    pub name: String,
    pub fields: Vec<(String, Kind, usize)>,
}

/// Field offsets and total size for a pointer size.
#[derive(Clone, Debug)]
pub struct Layout {
    pub fields: Vec<(String, usize)>,
    pub size: usize,
    pub align: usize,
}

fn scalar(ty: &str) -> Option<usize> {
    Some(match ty {
        "uint8_t" => 1,
        "uint32_t" | "int32_t" | "float" => 4,
        "uint64_t" => 8,
        _ => return None,
    })
}

impl Struct {
    /// Natural alignment (`uint64_t` aligned to 8, as on i686 Windows).
    pub fn layout(&self, pointer: usize) -> Layout {
        let all = all_structs();
        let mut at: usize = 0;
        let mut align: usize = 1;
        let mut fields = Vec::new();
        for (name, kind, count) in &self.fields {
            let (size, a) = match kind {
                Kind::Scalar(n) => (*n, *n),
                Kind::Pointer => (pointer, pointer),
                Kind::Nested(inner) => {
                    let l = all
                        .iter()
                        .find(|s| &s.name == inner)
                        .unwrap()
                        .layout(pointer);
                    (l.size, l.align)
                }
            };
            at = at.next_multiple_of(a);
            align = align.max(a);
            fields.push((name.clone(), at));
            at += size * count;
        }
        Layout {
            fields,
            size: at.next_multiple_of(align),
            align,
        }
    }
}

fn parse_decl(decl: &str, out: &mut Vec<(String, Kind, usize)>) {
    let decl = decl.trim();
    if decl.is_empty() {
        return;
    }
    if let Some(open) = decl.find("(") {
        // Function pointer: `ret (FF13_CDECL *name)(args)`.
        let inner = &decl[open + 1..];
        let name = inner[..inner.find(')').unwrap()]
            .trim_start_matches("FF13_CDECL")
            .trim()
            .trim_start_matches('*')
            .trim();
        out.push((name.into(), Kind::Pointer, 1));
        return;
    }
    let cleaned = decl.replace("const ", "").replace("struct ", "");
    let mut parts = cleaned.splitn(2, char::is_whitespace);
    let base = parts.next().unwrap().to_string();
    for declarator in parts.next().unwrap().split(',') {
        let d = declarator.trim();
        let pointer = d.contains('*');
        let d = d
            .trim_start_matches(['*', ' '])
            .trim_start_matches("const")
            .trim_start_matches(['*', ' ']);
        let (name, count) = match d.find('[') {
            Some(b) => (&d[..b], d[b + 1..d.len() - 1].parse().unwrap()),
            None => (d, 1),
        };
        let kind = if pointer || matches!(base.as_str(), "size_t" | "uintptr_t") {
            Kind::Pointer
        } else if let Some(n) = scalar(&base) {
            Kind::Scalar(n)
        } else if base.ends_with("Fn")
            || base.ends_with("Query")
            || base.ends_with("Callback")
            || base.ends_with("Visitor")
        {
            Kind::Pointer
        } else {
            Kind::Nested(base.clone())
        };
        out.push((name.trim().into(), kind, count));
    }
}

/// Every `typedef struct NAME { ... } NAME;` of the headers.
pub fn all_structs() -> Vec<Struct> {
    let text = strip_comments(&text());
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("typedef struct ") {
        rest = &rest[at + "typedef struct ".len()..];
        let open = rest.find('{').unwrap();
        let name = rest[..open].trim().to_string();
        let close = rest.find('}').unwrap();
        let mut fields = Vec::new();
        for decl in rest[open + 1..close].split(';') {
            parse_decl(decl, &mut fields);
        }
        out.push(Struct { name, fields });
        rest = &rest[close..];
    }
    out
}

/// Numeric object-like `#define`s (include guards and macros skipped).
pub fn defines() -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for line in strip_comments(&text()).lines() {
        let Some(rest) = line.trim().strip_prefix("#define ") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name.contains('(') {
            continue;
        }
        let v = value.trim_matches(['(', ')']).trim_end_matches('u');
        let parsed = if v == "UINT32_MAX" {
            Some(u32::MAX.into())
        } else if let Some(hex) = v.strip_prefix("0x") {
            i64::from_str_radix(hex, 16).ok()
        } else {
            v.parse().ok()
        };
        if let Some(value) = parsed {
            out.insert(name.to_string(), value);
        }
    }
    out
}

/// The C11 `_Static_assert`s on `FF13HostContext`: `(field, value)`,
/// with `sizeof` for the size assertion.
pub fn static_asserts() -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for line in HOST.lines().filter(|l| l.starts_with("_Static_assert(")) {
        let value = |s: &str| s.split(',').next().unwrap().trim().parse().unwrap();
        if let Some(rest) = line.strip_prefix("_Static_assert(offsetof(FF13HostContext, ") {
            let (field, tail) = rest.split_once(") == ").unwrap();
            out.push((field.to_string(), value(tail)));
        } else if let Some(rest) = line.strip_prefix("_Static_assert(sizeof(FF13HostContext) == ") {
            out.push(("sizeof".into(), value(rest)));
        }
    }
    out
}

#[test]
fn parser_handles_every_declaration_form() {
    let mut f = Vec::new();
    parse_decl("const char *const *options", &mut f);
    parse_decl("float reserved_color[4]", &mut f);
    parse_decl("uint32_t size, schema", &mut f);
    parse_decl("const struct FF13SettingExtension *next", &mut f);
    parse_decl("int32_t (FF13_CDECL *flush)(void)", &mut f);
    parse_decl("FF13SettingExtension header", &mut f);
    parse_decl("  ", &mut f);
    let names: Vec<&str> = f.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "options",
            "reserved_color",
            "size",
            "schema",
            "next",
            "flush",
            "header"
        ]
    );
    assert_eq!(f[1].1, Kind::Scalar(4));
    assert_eq!(f[1].2, 4);
    assert_eq!(f[6].1, Kind::Nested("FF13SettingExtension".into()));
    assert_eq!(strip_comments("a /* b */ c // d\ne /* open"), "a  c \ne ");
    assert!(all_structs().len() >= 15);
}

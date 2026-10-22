//! Derive macros for `workshop-ext-sdk`.
//!
//! `#[derive(PackedRecord)]` implements `workshop_ext_sdk::packed::PackedRecord`
//! for a struct with named fields:
//!
//! ```ignore
//! #[derive(PackedRecord)]
//! #[packed(version = 1, schema = 2, migrate = from_schema_1, valid = check)]
//! struct Options {
//!     #[packed(default = 1, max = 11)]
//!     destination: u8,                 // key "Destination", 4 bits
//!     #[packed(key = "BattleId", default = 20217, min = 1, max = 29999)]
//!     battle: u16,
//!     verbose_log: bool,               // key "VerboseLog", 1 bit
//!     #[packed(text)]
//!     text: Vec<workshop_ext_sdk::packed::TextEntry>,
//! }
//! ```
//!
//! Container attributes: `version` (format byte, default 1), `schema`
//! (host storage schema, default 1), `migrate = path` (`fn(u32, &[u8]) ->
//! Option<Self>`), `valid = path` (`fn(&Self) -> bool`). Field attributes:
//! `key` (default: the field name in PascalCase), `default` (default:
//! `min`), `min` (default 0), `max` (required except for `bool`), all
//! constant expressions; `text` marks the one `Vec<TextEntry>` text tail.
//! Field order is the wire order and must never change once released.
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::quote;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Error, Expr, Fields, Ident, LitStr, Token, Type,
    parse_macro_input,
};

/// Implements `PackedRecord` (see the crate documentation).
#[proc_macro_derive(PackedRecord, attributes(packed))]
pub fn derive_packed_record(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// `battle_id` → `BattleId`.
fn pascal(name: &str) -> String {
    name.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// The `#[packed(...)]` arguments of `attrs`: `name = expression` or a bare
/// `name`, each one of `allowed`.
fn args(attrs: &[Attribute], allowed: &[&str]) -> syn::Result<Vec<(String, Option<Expr>)>> {
    let mut out = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("packed")) {
        attr.parse_nested_meta(|meta| {
            let name = meta
                .path
                .get_ident()
                .map(Ident::to_string)
                .unwrap_or_default();
            if !allowed.contains(&name.as_str()) {
                return Err(meta.error(format!("expected one of: {allowed:?}")));
            }
            let value = if meta.input.peek(Token![=]) {
                Some(meta.value()?.parse()?)
            } else {
                None
            };
            out.push((name, value));
            Ok(())
        })?;
    }
    Ok(out)
}

/// The last value given for `name`.
fn arg(args: &[(String, Option<Expr>)], name: &str) -> Option<Tokens> {
    let value = args.iter().rev().find(|a| a.0 == name)?.1.as_ref()?;
    Some(quote!(#value))
}

const NOT_A_RECORD: &str = "PackedRecord: only non-generic structs with named fields";
const TWO_TEXTS: &str = "PackedRecord: only one #[packed(text)] field";
const NO_MAX: &str = "PackedRecord: numeric fields need `max`";

fn is_bool(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("bool"))
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let named = match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(named),
            ..
        }) if input.generics.params.is_empty() => named,
        _ => return Err(Error::new_spanned(input, NOT_A_RECORD)),
    };
    let c = args(&input.attrs, &["version", "schema", "migrate", "valid"])?;
    let sdk = quote!(::workshop_ext_sdk::packed);
    let mut layout = Vec::new();
    let mut values = Vec::new();
    let mut parts = Vec::new();
    let mut text_field: Option<Ident> = None;
    let mut index = 0usize;
    for field in &named.named {
        let ident = field.ident.clone().expect("named field");
        let attr = args(&field.attrs, &["key", "default", "min", "max", "text"])?;
        if attr.iter().any(|a| a.0 == "text") {
            if text_field.replace(ident.clone()).is_some() {
                return Err(Error::new_spanned(field, TWO_TEXTS));
            }
            parts.push(quote!(#ident: text));
            continue;
        }
        let key = arg(&attr, "key").unwrap_or_else(|| {
            let key = LitStr::new(&pascal(&ident.to_string()), Span::call_site());
            quote!(#key)
        });
        let min = arg(&attr, "min").unwrap_or(quote!(0));
        let max = arg(&attr, "max").or_else(|| is_bool(&field.ty).then(|| quote!(1)));
        let Some(max) = max else {
            return Err(Error::new_spanned(field, NO_MAX));
        };
        let default = arg(&attr, "default").unwrap_or_else(|| min.clone());
        layout
            .push(quote!(#sdk::Field::new(#key, (#default) as i64, (#min) as i64, (#max) as i64)));
        values.push(quote!(#sdk::PackedValue::to_packed(self.#ident)));
        parts.push(quote!(#ident: #sdk::PackedValue::from_packed(
            values.get(#index).copied().unwrap_or(Self::LAYOUT.fields[#index].default)
        )));
        index += 1;
    }
    let name = &input.ident;
    let version = arg(&c, "version").unwrap_or(quote!(1));
    let schema = arg(&c, "schema").unwrap_or(quote!(1));
    let text = match &text_field {
        Some(ident) => quote!(&self.#ident),
        None => quote!(&[]),
    };
    let unused_text = text_field.is_none().then(|| quote!(let _ = text;));
    let migrate = arg(&c, "migrate").map(|path| {
        quote!(fn migrate(schema: u32, bytes: &[u8]) -> ::core::option::Option<Self> {
            #path(schema, bytes)
        })
    });
    let valid = arg(&c, "valid").map(|path| quote!(fn is_valid(&self) -> bool { #path(self) }));
    Ok(quote! {
        impl #sdk::PackedRecord for #name {
            const LAYOUT: #sdk::Layout = #sdk::Layout {
                version: #version,
                schema: #schema,
                fields: &[#(#layout),*],
            };
            fn values(&self) -> ::std::vec::Vec<i64> {
                ::std::vec![#(#values),*]
            }
            fn text(&self) -> &[#sdk::TextEntry] {
                #text
            }
            fn from_parts(values: &[i64], text: ::std::vec::Vec<#sdk::TextEntry>) -> Self {
                #unused_text
                Self { #(#parts),* }
            }
            #migrate
            #valid
        }
        const _: () = ::core::assert!(
            <#name as #sdk::PackedRecord>::LAYOUT.is_valid(),
            "PackedRecord: every field needs min <= default <= max and a unique key"
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        let input: DeriveInput = syn::parse_str(src).unwrap();
        match expand(&input) {
            Ok(t) => t.to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    #[test]
    fn expands_fields_text_and_hooks() {
        let out = run(
            "#[packed(version = 2, schema = 3, migrate = up, valid = ok)] struct R { \
             #[packed(default = 1, max = 11)] destination: u8, \
             #[packed(key = \"Id\", min = 1, max = 9)] battle_id: u16, \
             verbose_log: bool, #[packed(text)] text: Vec<TextEntry> }",
        );
        for want in [
            "\"Destination\"",
            "\"Id\"",
            "\"VerboseLog\"",
            "fn migrate",
            "fn is_valid",
            "version : 2",
            "schema : 3",
        ] {
            assert!(out.contains(want), "{want} in {out}");
        }
        let plain = run("struct P { flag: bool }");
        assert!(plain.contains("let _ = text"));
        assert_eq!(pascal("battle__id_"), "BattleId");
    }

    #[test]
    fn rejects_malformed_inputs() {
        for (src, want) in [
            ("struct G<T> { a: T }", "non-generic"),
            ("enum E { A }", "only non-generic structs"),
            ("struct T(u8);", "named fields"),
            ("struct N { a: u8 }", "need `max`"),
            (
                "struct D { #[packed(text)] a: Vec<u8>, #[packed(text)] b: Vec<u8> }",
                "only one",
            ),
            (
                "#[packed(bogus = 1)] struct B { a: bool }",
                "expected one of: [\"version\"",
            ),
            (
                "struct F { #[packed(bogus)] a: bool }",
                "expected one of: [\"key\"",
            ),
            (
                "struct K { #[packed(a::b)] a: bool }",
                "expected one of: [\"key\"",
            ),
        ] {
            assert!(run(src).contains(want), "{src}: {}", run(src));
        }
    }
}

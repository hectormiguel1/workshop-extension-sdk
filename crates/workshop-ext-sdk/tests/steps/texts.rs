//! Steps of `texts.feature`.
use super::Sdk;
use cucumber::then;
use std::borrow::Cow;
use workshop_ext_sdk::param::Param;
use workshop_ext_sdk::settings::{Builder, Meta};
use workshop_ext_sdk::texts::{self, Entry, LOCALES};

const GIL: Entry = (
    "Gil",
    [
        "Gil", "Gil", "Gils", "Gil", "Guil", "Guiles", "ギル", "Гили", "金幣", "金币", "길",
    ],
);

#[then(expr = "the key of {string} is {string}")]
fn key_of(_: &mut Sdk, source: String, key: String) {
    assert_eq!(texts::key(&source), key);
}

#[then("placeholders are the sorted single digits in braces")]
fn placeholders(_: &mut Sdk) {
    assert_eq!(texts::placeholders("{1} {0} {x} {10} {1}"), b"011".to_vec());
}

#[then(
    "a provider's looked-up strings are its short name, labels, help, options and affixes without status rows"
)]
fn required(_: &mut Sdk) {
    static LEVEL: Param = Param::new("Level", 1, 0, 9);
    static MODE: Param = Param::new("Mode", 0, 0, 1);
    let p = Builder::new()
        .status("st", "Status", || Cow::Borrowed("x"))
        .section("s", "Section", "Help.")
        .param_integer("l", "Level", "Help.", 1, &LEVEL)
        .display("Lv ", "%", 0)
        .param_combo("m", "Mode", "", &["A", "B"], &MODE)
        .finish(&Meta {
            id: "a.b",
            name: "FF13 Thing",
            author: "x",
            release: "1",
            help: "",
            website: None,
        });
    assert_eq!(
        unsafe { texts::required(p, &["Gil", "A"]) },
        [
            "Thing", "Section", "Help.", "Level", "Lv ", "%", "Mode", "A", "B", "Gil"
        ]
    );
}

#[then("a complete catalog has no problems, a missing, empty or drifted entry is reported")]
fn problems(_: &mut Sdk) {
    assert!(texts::problems(&["Gil"], &[GIL]).is_empty());
    assert_eq!(texts::problems(&["Nope"], &[GIL]).len(), 1);
    let mut e = GIL;
    e.1[0] = "";
    assert_eq!(texts::problems(&["Gil"], &[e]), ["\"Gil\": empty us"]);
    let d: Entry = (
        "{0} x",
        [
            "{0} x", "x", "{0}", "{0}", "{0}", "{0}", "{0}", "{0}", "{0}", "{0}", "{0}",
        ],
    );
    assert_eq!(
        texts::problems(&["{0} x"], &[d]),
        ["\"{0} x\": uk placeholders differ"]
    );
}

#[then("a container dump missing a locale fails the check")]
fn dump(_: &mut Sdk) {
    let k = texts::key("Gil");
    let rows: String = LOCALES
        .iter()
        .filter(|l| **l != "kr")
        .map(|l| format!("{k}\t{l}\tGil\n"))
        .collect();
    assert_eq!(
        texts::missing_in(&["Gil"], &rows),
        [format!("{k} kr (\"Gil\")")]
    );
}

#[then("the catalog is exported as escaped JSON lines")]
fn json(_: &mut Sdk) {
    let q: Entry = ("a\"\\\n\u{2}", ["y"; 11]);
    let out = texts::json_lines(&["Gil", "a\"\\\n\u{2}", "none"], &[GIL, q]);
    assert_eq!(out.lines().count(), 2);
    assert!(out.contains("\\\"\\\\\\n\\u0002"));
}

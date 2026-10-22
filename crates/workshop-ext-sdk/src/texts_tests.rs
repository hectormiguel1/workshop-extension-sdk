use super::*;
use crate::param::Param;
use crate::settings::{Builder, Meta};

const GIL: Entry = (
    "Gil",
    [
        "Gil", "Gil", "Gils", "Gil", "Guil", "Guiles", "ギル", "Гили", "金幣", "金币", "길",
    ],
);
const PREFIX: Entry = ("Gil {0} | ", ["Gil {0} | "; 11]);

#[test]
fn keys_are_the_menu_adapter_hash() {
    assert_eq!(key("Gil"), "$ms8b1998869903");
    assert_eq!(key("Gil {0} | "), "$ms18edac9230c3");
    assert_eq!(key("[R] "), "$ms2d66773c886d");
    assert_eq!(key("%"), "$ms984c86017900");
    assert_eq!(key("").len(), 15);
}

#[test]
fn placeholders_are_sorted_digits() {
    assert_eq!(placeholders("{1} of {0} {x} {12} {1}"), b"011".to_vec());
    assert!(placeholders("none").is_empty());
}

#[test]
fn required_lists_the_short_name_menu_and_runtime_texts() {
    static ON: Param = Param::flag("On", false);
    let p = Builder::new()
        .section("s", "Section", "")
        .param_checkbox("on", "On", "Help.", &ON)
        .finish(&Meta {
            id: "x.y",
            name: "FF13 Thing",
            author: "a",
            release: "1",
            help: "",
            website: None,
        });
    assert_eq!(
        unsafe { required(p, &["Gil", "On", ""]) },
        ["Thing", "Section", "On", "Help.", "Gil"]
    );
}

#[test]
fn problems_report_missing_empty_and_placeholder_drift() {
    assert!(problems(&["Gil", "Gil {0} | "], &[GIL, PREFIX]).is_empty());
    assert_eq!(problems(&["Nope"], &[GIL]), ["missing: \"Nope\""]);
    let mut empty = GIL;
    empty.1[7] = "";
    assert_eq!(problems(&["Gil"], &[empty]), ["\"Gil\": empty ru"]);
    let mut drift = PREFIX;
    drift.1[2] = "Gils | ";
    assert_eq!(
        problems(&["Gil {0} | "], &[drift]),
        ["\"Gil {0} | \": fr placeholders differ"]
    );
}

#[test]
fn missing_in_reads_a_text_dump() {
    let k = key("Gil");
    let mut rows = String::from("key\tlocale\ttext\n");
    for l in LOCALES {
        rows.push_str(&format!("{k}\t{l}\tGil\n"));
    }
    assert!(missing_in(&["Gil"], &rows).is_empty());
    let partial = rows.replace(&format!("{k}\tjp\tGil"), &format!("{k}\tjp\t "));
    assert_eq!(
        missing_in(&["Gil"], &partial),
        [format!("{k} jp (\"Gil\")")]
    );
    assert_eq!(missing_in(&["Gil"], "garbage").len(), 11);
}

#[test]
fn json_lines_escape_and_skip_unknown_sources() {
    let quoted: Entry = ("a \"b\"\\\n\u{1}", ["x"; 11]);
    let out = json_lines(&["Gil", "a \"b\"\\\n\u{1}", "unknown"], &[GIL, quoted]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].starts_with(
            "{\"key\":\"$ms8b1998869903\",\"source\":\"Gil\",\"texts\":{\"us\":\"Gil\""
        )
    );
    assert!(lines[0].contains("\"jp\":\"ギル\""));
    assert!(lines[1].contains("\"source\":\"a \\\"b\\\"\\\\\\n\\u0001\""));
}

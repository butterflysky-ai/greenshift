use super::*;

#[test]
fn escape_file_text_leaves_quotes_alone() {
    assert_eq!(escape_file_text("a\"b'c\u{1b}\\"), "a\"b'c\\u{1b}\\\\");
}

#[test]
fn tainted_displays_control_bytes_escaped() {
    for (raw, shown) in [
        ("\u{1b}[31mevil", r"\u{1b}[31mevil"),
        ("nul\0byte", r"nul\0byte"),
        ("bell\u{7}del\u{7f}", r"bell\u{7}del\u{7f}"),
        ("line\nfeed\rtab\t", r"line\nfeed\rtab\t"),
        ("\u{202e}evil", r"\u{202e}evil"),
        ("back\\slash", r"back\\slash"),
    ] {
        let text = Tainted::from(raw).to_string();
        assert_eq!(text, shown, "{raw:?}");
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }
}

#[test]
fn tainted_displays_printable_text_and_quotes_unchanged() {
    let raw = "widgets::parses \"a\" 'sprocket' & caf\u{e9} \u{263a}";
    assert_eq!(Tainted::from(raw).to_string(), raw);
}

#[test]
fn tainted_debug_escapes_control_bytes() {
    let text = format!("{:?}", Tainted::from("\u{1b}[31mevil\0"));
    assert!(!text.chars().any(char::is_control), "{text:?}");
    assert!(text.contains(r"\u{1b}[31mevil\0"), "{text:?}");
}

#[test]
fn tainted_as_raw_is_the_text_unchanged() {
    let raw = "\u{1b}[31mevil\0\"q\"";
    assert_eq!(Tainted::from(raw).as_raw(), raw);
    assert_eq!(Tainted::from(raw.to_string()).as_raw(), raw);
}

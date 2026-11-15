use super::*;
use crate::tainted::Tainted;
use std::collections::BTreeSet;

#[test]
fn escape_leaves_ordinary_text_alone() {
    for text in [
        "widgets::parses_a_sprocket",
        "a (b | c & d",
        "not all( - none(",
        " leading and trailing ",
        "back\"quote'",
        "uni::ünï ✓",
        "#glob*",
        "~contains",
        "=equal",
    ] {
        assert_eq!(escape(text), text);
    }
}

#[test]
fn escape_marks_what_ends_or_escapes_a_matcher() {
    for (text, want) in [
        (")", r"\)"),
        (",", r"\,"),
        (r"\", r"\\"),
        ("/", r"\/"),
        ("x) | all() | test(=y", r"x\) | all(\) | test(=y"),
        ("a, b", r"a\, b"),
        (r"back\slash/x", r"back\\slash\/x"),
    ] {
        assert_eq!(escape(text), want, "{text:?}");
    }
}

#[test]
fn escape_keeps_control_characters_off_the_line() {
    for (text, want) in [
        ("a\nb", r"a\nb"),
        ("a\rb", r"a\rb"),
        ("a\tb", r"a\tb"),
        ("a\u{1b}[31m", r"a\u{1b}[31m"),
        ("\u{0}", r"\u{0}"),
        ("\u{7f}", r"\u{7f}"),
        ("\u{85}", r"\u{85}"),
    ] {
        assert_eq!(escape(text), want, "{text:?}");
    }
}

#[test]
fn term_matches_binary_and_test_exactly() {
    assert_eq!(
        term("greenshift::cli", "select_needs_a_store"),
        "(binary_id(=greenshift::cli) & test(=select_needs_a_store))"
    );
    assert_eq!(
        term("odd-names::odd", "a (b) | c & d, e"),
        r"(binary_id(=odd-names::odd) & test(=a (b\) | c & d\, e))"
    );
}

#[test]
fn term_without_a_suite_matches_the_name_alone() {
    // An empty `=` matcher is not a valid filterset.
    assert_eq!(term("", "lonely"), "test(=lonely)");
}

#[test]
fn any_of_joins_terms_and_matches_nothing_when_empty() {
    assert_eq!(any_of([]), "none()");
    assert_eq!(any_of([("b", "t")]), "(binary_id(=b) & test(=t))");
    assert_eq!(
        any_of([("b", "t"), ("c", "u,v")]),
        r"(binary_id(=b) & test(=t)) | (binary_id(=c) & test(=u\,v))"
    );
}

#[test]
fn all_but_negates_and_matches_everything_when_empty() {
    assert_eq!(all_but([]), "all()");
    assert_eq!(all_but([("b", "t")]), "not ((binary_id(=b) & test(=t)))");
    assert_eq!(
        all_but([("b", "t"), ("c", "u")]),
        "not ((binary_id(=b) & test(=t)) | (binary_id(=c) & test(=u)))"
    );
}

fn bins(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn line<'a>(class: Class, suite: &'a Tainted, name: &'a Tainted) -> Line<'a> {
    Line {
        class,
        coverage: 100.0,
        confidence: 100.0,
        suite,
        name,
    }
}

#[test]
fn filter_rich_is_everything_but_poor() {
    let (b, r, p, q) = (
        Tainted::from("bin"),
        Tainted::from("rich_one"),
        Tainted::from("poor_one"),
        Tainted::from("poor_two"),
    );
    let lines = [
        line(Class::Rich, &b, &r),
        line(Class::Poor, &b, &p),
        line(Class::Poor, &b, &q),
    ];
    assert_eq!(
        filter(&lines, Class::Rich, &bins(&["bin"])),
        "not ((binary_id(=bin) & test(=poor_one)) | (binary_id(=bin) & test(=poor_two)))"
    );
    assert_eq!(
        filter(&lines, Class::Poor, &bins(&["bin"])),
        "(binary_id(=bin) & test(=poor_one)) | (binary_id(=bin) & test(=poor_two))"
    );
}

#[test]
fn filter_with_no_poor_line() {
    let (b, r) = (Tainted::from("bin"), Tainted::from("rich_one"));
    let lines = [line(Class::Rich, &b, &r)];
    assert_eq!(filter(&lines, Class::Rich, &bins(&["bin"])), "all()");
    assert_eq!(filter(&lines, Class::Poor, &bins(&["bin"])), "none()");
    assert_eq!(filter(&[], Class::Rich, &bins(&[])), "all()");
    assert_eq!(filter(&[], Class::Poor, &bins(&[])), "none()");
}

#[test]
fn filter_leaves_out_binaries_not_in_the_inventory() {
    // nextest refuses a whole filterset in which a binary_id(=...) matches
    // no binary.
    let (b, gone, none, p, q, r) = (
        Tainted::from("bin"),
        Tainted::from("gone"),
        Tainted::from(""),
        Tainted::from("poor_one"),
        Tainted::from("poor_two"),
        Tainted::from("rich_one"),
    );
    let lines = [
        line(Class::Rich, &gone, &r),
        line(Class::Poor, &gone, &p),
        line(Class::Poor, &b, &q),
        line(Class::Poor, &none, &p),
    ];
    let inventory = bins(&["bin", "other"]);
    assert_eq!(
        filter(&lines, Class::Poor, &inventory),
        "(binary_id(=bin) & test(=poor_two)) | test(=poor_one)"
    );
    assert_eq!(
        filter(&lines, Class::Rich, &inventory),
        "not ((binary_id(=bin) & test(=poor_two)) | test(=poor_one))"
    );
    let only_gone = [line(Class::Rich, &gone, &r), line(Class::Poor, &gone, &p)];
    assert_eq!(filter(&only_gone, Class::Poor, &inventory), "none()");
    assert_eq!(filter(&only_gone, Class::Rich, &inventory), "all()");
}

/// Trimmed `cargo nextest list --message-format json` output from 0.9.143.
const FULL_LIST: &str = r#"{"rust-build-meta":{"target-directory":"/t","base-output-directories":["debug"],"non-test-binaries":{},"linked-paths":[]},"test-count":2,"rust-suites":{"greenshift":{"package-name":"greenshift","binary-id":"greenshift","binary-name":"greenshift","kind":"lib","status":"listed","testcases":{"cli::tests::ingest_options":{"kind":"test","ignored":false,"filter-match":{"status":"matches"}}}},"greenshift::cli":{"package-name":"greenshift","binary-id":"greenshift::cli","binary-name":"cli","kind":"test","status":"listed","testcases":{}}}}"#;

/// Trimmed `cargo nextest list --list-type binaries-only --message-format
/// json` output from 0.9.143.
const BINARIES_ONLY: &str = r#"{"rust-build-meta":{"target-directory":"/t"},"rust-binaries":{"greenshift":{"binary-id":"greenshift","binary-name":"greenshift","kind":"lib","build-platform":"target"},"greenshift::bin/greenshift":{"binary-id":"greenshift::bin/greenshift","binary-name":"greenshift","kind":"bin","build-platform":"target"}}}"#;

#[test]
fn binary_ids_from_a_full_or_binaries_only_listing() {
    assert_eq!(
        binary_ids(FULL_LIST).unwrap(),
        bins(&["greenshift", "greenshift::cli"])
    );
    assert_eq!(
        binary_ids(BINARIES_ONLY).unwrap(),
        bins(&["greenshift", "greenshift::bin/greenshift"])
    );
    assert_eq!(binary_ids(r#"{"rust-binaries":{}}"#).unwrap(), bins(&[]));
}

#[test]
fn binary_ids_refuses_anything_else() {
    for bad in [
        "",
        "not json",
        "[]",
        "{}",
        r#"{"rust-suites":[]}"#,
        r#"{"rust-binaries":{"x":{"binary-name":"x"}}}"#,
        r#"{"rust-binaries":{"x":{"binary-id":7}}}"#,
    ] {
        assert!(binary_ids(bad).is_err(), "{bad:?}");
    }
}

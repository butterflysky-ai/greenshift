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
    Line::new(class, 100.0, 100.0, suite, name)
}

#[test]
fn filter_rich_is_everything_but_poor() {
    let (b, r, p, q) = (
        Tainted::raw("bin"),
        Tainted::raw("rich_one"),
        Tainted::raw("poor_one"),
        Tainted::raw("poor_two"),
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
    let (b, r) = (Tainted::raw("bin"), Tainted::raw("rich_one"));
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
        Tainted::raw("bin"),
        Tainted::raw("gone"),
        Tainted::raw(""),
        Tainted::raw("poor_one"),
        Tainted::raw("poor_two"),
        Tainted::raw("rich_one"),
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
        binary_ids(FULL_LIST).unwrap().as_set(),
        &bins(&["greenshift", "greenshift::cli"])
    );
    assert_eq!(
        binary_ids(BINARIES_ONLY).unwrap().as_set(),
        &bins(&["greenshift", "greenshift::bin/greenshift"])
    );
    assert_eq!(
        binary_ids(r#"{"rust-binaries":{}}"#).unwrap().as_set(),
        &bins(&[])
    );
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

#[test]
fn a_full_listing_names_its_binaries_tests() {
    let listing = listing(FULL_LIST).unwrap();
    assert_eq!(
        listing.binaries().as_set(),
        &bins(&["greenshift", "greenshift::cli"])
    );
    assert!(listing.has("greenshift", "cli::tests::ingest_options"));
    assert!(!listing.has("greenshift", "cli::tests::gone"));
    // A listed binary with no tests has none to keep.
    assert!(!listing.has("greenshift::cli", "anything"));
    assert!(!listing.has("gone", "cli::tests::ingest_options"));
    // A test with no suite matches by name in any binary.
    assert!(listing.has("", "anything"));
}

#[test]
fn a_binaries_only_listing_keeps_every_test_of_its_binaries() {
    let listing = listing(BINARIES_ONLY).unwrap();
    assert!(listing.tests().is_empty());
    assert!(listing.has("greenshift", "any::name"));
    assert!(!listing.has("gone", "any::name"));
}

#[test]
fn listing_refuses_testcases_that_are_not_an_object() {
    let bad = r#"{"rust-suites":{"x":{"binary-id":"x","status":"listed","testcases":[]}}}"#;
    assert!(listing(bad).is_err());
}

#[test]
fn a_binary_a_full_listing_skipped_keeps_every_test() {
    // nextest lists a binary its -E filters out as skipped, with no
    // testcases: its tests are unknown, not gone.
    let listing =
        listing(r#"{"rust-suites":{"x":{"binary-id":"x","status":"skipped","testcases":{}}}}"#)
            .unwrap();
    assert!(listing.has("x", "any::name"));
    assert!(!listing.has("y", "any::name"));
}

/// The `(suite, name)` of each test, raw.
fn raw(tests: &[crate::plan::InventoryTest]) -> Vec<(&str, &str)> {
    tests
        .iter()
        .map(|t| (t.suite().as_raw(), t.name().as_raw()))
        .collect()
}

#[test]
fn selected_tests_are_the_matching_testcases_of_listed_binaries() {
    let json = r#"{"rust-suites":{
        "b::odd":{"binary-id":"b::odd","status":"listed","testcases":{
            "a (b), c":{"kind":"test","ignored":false,"filter-match":{"status":"matches"}},
            "left::out":{"kind":"test","ignored":false,"filter-match":{"status":"mismatch","reason":"expression"}},
            "ignored":{"kind":"test","ignored":true,"filter-match":{"status":"mismatch","reason":"ignored"}}}},
        "a::plain":{"binary-id":"a::plain","status":"listed","testcases":{
            "z":{"filter-match":{"status":"matches"}},
            "m":{"filter-match":{"status":"matches"}}}},
        "c::skipped":{"binary-id":"c::skipped","status":"skipped","testcases":{}}}}"#;
    assert_eq!(
        raw(&selected_tests(json).unwrap()),
        [("a::plain", "m"), ("a::plain", "z"), ("b::odd", "a (b), c")]
    );
    assert_eq!(
        raw(&selected_tests(FULL_LIST).unwrap()),
        [("greenshift", "cli::tests::ingest_options")]
    );
}

#[test]
fn selected_tests_needs_a_full_listing() {
    // A binaries-only listing names no tests, so it selects none of them.
    for bad in [
        BINARIES_ONLY,
        "not json",
        "{}",
        r#"{"rust-suites":[]}"#,
        r#"{"rust-suites":{"x":{"status":"listed","testcases":{}}}}"#,
        r#"{"rust-suites":{"x":{"binary-id":"x","status":"listed","testcases":[]}}}"#,
        r#"{"rust-suites":{"x":{"binary-id":"x","status":"listed","testcases":{"t":{}}}}}"#,
    ] {
        assert!(selected_tests(bad).is_err(), "{bad:?}");
    }
}

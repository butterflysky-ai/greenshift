use super::*;
use crate::select::{Class, Line};
use crate::tainted::Tainted;

fn id(suite: &str, name: &str) -> InventoryTest {
    InventoryTest::new(Tainted::raw(suite), Tainted::raw(name))
}

/// Stored tests, as `select`'s lines borrow them.
struct Stored(Vec<(Class, Tainted, Tainted)>);

impl Stored {
    fn new(tests: &[(Class, &str, &str)]) -> Stored {
        Stored(
            tests
                .iter()
                .map(|&(class, suite, name)| (class, Tainted::raw(suite), Tainted::raw(name)))
                .collect(),
        )
    }

    fn lines(&self) -> Vec<Line<'_>> {
        self.0
            .iter()
            .map(|(class, suite, name)| Line::new(*class, 100.0, 100.0, suite, name))
            .collect()
    }
}

fn ids(tests: &[&InventoryTest]) -> Vec<(String, String)> {
    tests
        .iter()
        .map(|t| (t.suite().as_raw().to_owned(), t.name().as_raw().to_owned()))
        .collect()
}

fn pairs(tests: &[(&str, &str)]) -> Vec<(String, String)> {
    tests
        .iter()
        .map(|&(s, n)| (s.to_owned(), n.to_owned()))
        .collect()
}

/// Every inventory test is in exactly one group, and nothing else is.
fn assert_partition(plan: &Plan<'_>, inventory: &[InventoryTest]) {
    let planned: Vec<&InventoryTest> = plan.rich().iter().chain(plan.poor()).copied().collect();
    let mut seen = ids(&planned);
    seen.sort();
    let mut want = ids(&inventory.iter().collect::<Vec<_>>());
    want.sort();
    want.dedup();
    assert_eq!(seen, want);
}

#[test]
fn parses_json_lines() {
    let text = "{\"suite\":\"s\",\"name\":\"a, b (c)\"}\n\
                \n\
                {\"name\": \"back\\\\slash\", \"suite\": \"\"}\r\n\
                {\"suite\":\"s\",\"name\":\"uni \\u00fc \u{2713}\",\"extra\":1}";
    assert_eq!(
        parse_inventory(text).unwrap().as_slice(),
        [
            id("s", "a, b (c)"),
            id("", r"back\slash"),
            id("s", "uni \u{fc} \u{2713}"),
        ]
    );
}

#[test]
fn an_empty_inventory_has_no_tests() {
    assert_eq!(parse_inventory("").unwrap().as_slice(), []);
    assert_eq!(parse_inventory("\n \n").unwrap().as_slice(), []);
}

#[test]
fn a_malformed_inventory_names_the_line() {
    for (text, line) in [
        ("not json", 1),
        ("{\"suite\":\"s\",\"name\":\"a\"}\n[\"s\",\"a\"]", 2),
        ("{\"suite\":\"s\"}", 1),
        ("{\"name\":\"a\"}", 1),
        ("{\"suite\":\"s\",\"name\":7}", 1),
        ("{\"suite\":null,\"name\":\"a\"}", 1),
        (
            "{\"suite\":\"s\",\"name\":\"a\"} {\"suite\":\"s\",\"name\":\"b\"}",
            1,
        ),
        ("\n\n{\"suite\":\"s\",\"name\":\"a\"", 3),
    ] {
        let err = parse_inventory(text).unwrap_err().to_string();
        assert!(
            err.starts_with(&format!("not a greenshift inventory: line {line}: ")),
            "{text:?}: {err}"
        );
        assert_eq!(err.matches("line").count(), 1, "two line numbers: {err}");
    }
}

#[test]
fn poor_is_the_poor_lines_in_the_inventory_and_rich_is_the_rest() {
    let stored = Stored::new(&[
        (Class::Rich, "s", "picked"),
        (Class::Poor, "s", "steady"),
        (Class::Poor, "s", "slow"),
    ]);
    let inventory = [
        id("s", "fresh"),
        id("s", "slow"),
        id("s", "steady"),
        id("s", "picked"),
        id("s", "under_observed"),
    ];
    let lines = stored.lines();
    let plan = plan(&lines, &inventory);
    // Rich in pick order, then the rest in inventory order; poor in
    // select's order.
    assert_eq!(
        ids(plan.rich()),
        pairs(&[("s", "picked"), ("s", "fresh"), ("s", "under_observed")])
    );
    assert_eq!(ids(plan.poor()), pairs(&[("s", "steady"), ("s", "slow")]));
    assert_partition(&plan, &inventory);
}

#[test]
fn rich_picks_keep_select_s_pick_order() {
    // Pick order is neither name nor inventory order.
    let stored = Stored::new(&[
        (Class::Rich, "s", "m_pick"),
        (Class::Rich, "s", "z_pick"),
        (Class::Rich, "s", "a_pick"),
        (Class::Poor, "s", "steady"),
    ]);
    let inventory = [
        id("s", "a_pick"),
        id("s", "fresh"),
        id("s", "steady"),
        id("s", "z_pick"),
        id("s", "m_pick"),
    ];
    let lines = stored.lines();
    let plan = plan(&lines, &inventory);
    assert_eq!(
        ids(plan.rich()),
        pairs(&[
            ("s", "m_pick"),
            ("s", "z_pick"),
            ("s", "a_pick"),
            ("s", "fresh")
        ])
    );
    assert_eq!(ids(plan.poor()), pairs(&[("s", "steady")]));
    assert_partition(&plan, &inventory);
}

#[test]
fn stored_tests_not_in_the_inventory_are_left_out() {
    let stored = Stored::new(&[
        (Class::Rich, "s", "gone_rich"),
        (Class::Poor, "s", "gone_poor"),
        (Class::Poor, "other", "kept"),
        (Class::Poor, "s", "kept"),
    ]);
    let inventory = [id("s", "kept")];
    let lines = stored.lines();
    let plan = plan(&lines, &inventory);
    assert!(plan.rich().is_empty());
    assert_eq!(ids(plan.poor()), pairs(&[("s", "kept")]));
    assert_partition(&plan, &inventory);
}

#[test]
fn with_no_lines_every_test_is_rich() {
    let inventory = [id("s", "a"), id("", "b")];
    let plan = plan(&[], &inventory);
    assert_eq!(ids(plan.rich()), pairs(&[("s", "a"), ("", "b")]));
    assert!(plan.poor().is_empty());
    assert_partition(&plan, &inventory);
}

#[test]
fn a_test_listed_twice_is_planned_once() {
    let stored = Stored::new(&[(Class::Poor, "s", "p")]);
    let inventory = [id("s", "p"), id("s", "r"), id("s", "p"), id("s", "r")];
    let lines = stored.lines();
    let plan = plan(&lines, &inventory);
    assert_eq!(ids(plan.rich()), pairs(&[("s", "r")]));
    assert_eq!(ids(plan.poor()), pairs(&[("s", "p")]));
    assert_partition(&plan, &inventory);
}

#[test]
fn an_empty_inventory_plans_nothing() {
    let stored = Stored::new(&[(Class::Rich, "s", "a"), (Class::Poor, "s", "b")]);
    let lines = stored.lines();
    let plan = plan(&lines, &[]);
    assert!(plan.rich().is_empty() && plan.poor().is_empty());
    assert_eq!(
        plan.to_json(HistoryState::Ok),
        r#"{"history":"ok","rich":[],"poor":[]}"#
    );
}

/// The `key` group of the plan `json`, read back as `(suite, name)`s.
fn read_group(json: &str, key: &str) -> Vec<(String, String)> {
    let plan: serde_json::Value = serde_json::from_str(json).unwrap();
    plan[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} array: {json}"))
        .iter()
        .map(|t| {
            (
                t["suite"].as_str().unwrap().to_owned(),
                t["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn names_are_matched_and_written_raw() {
    let odd = [
        ("s", "a, b (c)"),
        ("s", r"back\slash"),
        ("s", "tab\there"),
        ("s", "uni \u{fc}n\u{ef} \u{2713}"),
        ("s", "quote\"d"),
        ("s,(x)", "esc\u{1b}[31m"),
    ];
    let stored = Stored::new(&odd.map(|(s, n)| (Class::Poor, s, n)));
    let inventory: Vec<InventoryTest> = odd.iter().map(|&(s, n)| id(s, n)).collect();
    let lines = stored.lines();
    let plan = plan(&lines, &inventory);
    assert!(plan.rich().is_empty());
    assert_eq!(ids(plan.poor()), pairs(&odd));
    let json = plan.to_json(HistoryState::Ok);
    assert_eq!(read_group(&json, "poor"), pairs(&odd));
    assert!(!json.contains('\u{1b}'), "{json}");
}

#[test]
fn every_control_character_is_escaped_and_reads_back() {
    // DEL and C1 (CSI, NEL) are control characters that JSON may leave
    // raw; the plan escapes them all, as nextest's filterset does.
    let odd = [("s", "del\u{7f}"), ("s\u{9b}31m", "csi\u{9b}nel\u{85}x")];
    let inventory: Vec<InventoryTest> = odd.iter().map(|&(s, n)| id(s, n)).collect();
    let plan = plan(&[], &inventory);
    let json = plan.to_json(HistoryState::Ok);
    assert!(!json.chars().any(char::is_control), "{json:?}");
    assert!(json.contains(r"del\u007f"), "{json}");
    assert!(json.contains(r"csi\u009bnel\u0085x"), "{json}");
    assert_eq!(read_group(&json, "rich"), pairs(&odd));
}

#[test]
fn json_has_rich_then_poor_on_one_line() {
    let stored = Stored::new(&[(Class::Poor, "s", "p")]);
    let inventory = [id("s", "p"), id("", "r")];
    let lines = stored.lines();
    assert_eq!(
        plan(&lines, &inventory).to_json(HistoryState::Ok),
        r#"{"history":"ok","rich":[{"suite":"","name":"r"}],"poor":[{"suite":"s","name":"p"}]}"#
    );
}

#[test]
fn json_names_the_history_state_first() {
    let inventory = [id("s", "r")];
    let plan = plan(&[], &inventory);
    assert_eq!(
        plan.to_json(HistoryState::Error),
        r#"{"history":"error","rich":[{"suite":"s","name":"r"}],"poor":[]}"#
    );
}

#[test]
fn a_history_state_displays_as_the_plan_writes_it() {
    assert_eq!(HistoryState::Ok.to_string(), "ok");
    assert_eq!(HistoryState::Error.to_string(), "error");
}

#[test]
fn a_test_is_its_suite_and_name_not_its_name_alone() {
    // A poor name in another suite is a different test, new to the
    // store, so rich.
    let stored = Stored::new(&[(Class::Poor, "s", "same"), (Class::Rich, "s", "pick")]);
    let inventory = [id("t", "same"), id("s", "same"), id("t", "pick")];
    let lines = stored.lines();
    let plan = plan(&lines, &inventory);
    assert_eq!(ids(plan.rich()), pairs(&[("t", "same"), ("t", "pick")]));
    assert_eq!(ids(plan.poor()), pairs(&[("s", "same")]));
    assert_partition(&plan, &inventory);
}

#[test]
fn a_json_error_names_its_line_once_and_its_column() {
    let text = "{\"suite\":\"s\",\"name\":\"a\"}\n\n{\"suite\": x}";
    assert_eq!(
        parse_inventory(text).unwrap_err().to_string(),
        "not a greenshift inventory: line 3: expected value at column 11"
    );
}

#[test]
fn every_invisible_character_is_escaped_and_reads_back() {
    // Line and paragraph separators, bidi and other format characters,
    // one beyond the BMP (a surrogate pair in JSON): all invisible, all
    // escaped. Visible text, a combining mark included, is left as it is.
    let odd = [
        ("s\u{2028}", "para\u{2029}"),
        ("s", "bidi\u{202e}evil\u{2066}x\u{2069}"),
        ("s", "zw\u{200b}sp\u{feff}soft\u{ad}"),
        ("s", "tag\u{e0001}"),
        ("s", "nb\u{a0}sp"),
        ("s", "u\u{308} \u{2713}"),
    ];
    let inventory: Vec<InventoryTest> = odd.iter().map(|&(s, n)| id(s, n)).collect();
    let plan = plan(&[], &inventory);
    let json = plan.to_json(HistoryState::Ok);
    for escape in [
        r#""s\u2028","name":"para\u2029""#,
        r"bidi\u202eevil\u2066x\u2069",
        r"zw\u200bsp\ufeffsoft\u00ad",
        r"tag\udb40\udc01",
        r"nb\u00a0sp",
        "u\u{308} \u{2713}",
    ] {
        assert!(json.contains(escape), "{escape} not in {json}");
    }
    assert!(!json.chars().any(invisible), "{json:?}");
    assert_eq!(read_group(&json, "rich"), pairs(&odd));
}

#[test]
fn a_repeated_member_takes_its_last_value() {
    let text = r#"{"suite":"s","name":"first","name":"last","suite":"t"}"#;
    assert_eq!(parse_inventory(text).unwrap().as_slice(), [id("t", "last")]);
}

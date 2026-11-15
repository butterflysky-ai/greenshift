//! Runs `greenshift plan` on stores built by `greenshift ingest` from
//! generated JUnit, with names that `select` has to escape for display.

use std::collections::BTreeSet;
use std::path::PathBuf;

use assert_cmd::Command;

fn greenshift() -> Command {
    Command::cargo_bin("greenshift").expect("binary builds")
}

/// `text` as an XML attribute value.
fn attr(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out
}

/// A JUnit report of `tests` as `(suite, name, failed)`, each taking 5 ms.
fn junit(tests: &[(&str, &str, bool)]) -> String {
    let mut xml = String::from("<testsuites>\n<testsuite name=\"run\">\n");
    for &(suite, name, failed) in tests {
        xml.push_str(&format!(
            "<testcase classname=\"{}\" name=\"{}\" time=\"0.005\">{}</testcase>\n",
            attr(suite),
            attr(name),
            if failed { "<failure/>" } else { "" }
        ));
    }
    xml.push_str("</testsuite>\n</testsuites>\n");
    xml
}

/// `tests` as an inventory: one JSON object per line.
fn inventory(tests: &[(&str, &str)]) -> String {
    tests
        .iter()
        .map(|&(suite, name)| serde_json::json!({"suite": suite, "name": name}).to_string() + "\n")
        .collect()
}

const COMMA: &str = "a, b (c)";
const BACKSLASH: &str = r"back\slash/x";
const TAB: &str = "tab\there";
const UNI: &str = "uni \u{2713} \u{fc}n\u{ef}";

struct History {
    dir: tempfile::TempDir,
    db: PathBuf,
    runs: usize,
}

impl History {
    fn new() -> History {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("history.sqlite");
        History { dir, db, runs: 0 }
    }

    fn ingest(&mut self, tests: &[(&str, &str, bool)]) {
        self.runs += 1;
        let file = self.dir.path().join(format!("run{}.xml", self.runs));
        std::fs::write(&file, junit(tests)).unwrap();
        greenshift()
            .arg("ingest")
            .arg("--db")
            .arg(&self.db)
            .arg(&file)
            .assert()
            .success();
    }

    fn write_inventory(&self, text: &str) -> PathBuf {
        let path = self.dir.path().join("inventory.jsonl");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn plan(&self, inventory: &str, args: &[&str]) -> assert_cmd::assert::Assert {
        let path = self.write_inventory(inventory);
        greenshift()
            .arg("plan")
            .arg("--db")
            .arg(&self.db)
            .arg("--inventory")
            .arg(path)
            .args(args)
            .assert()
    }
}

/// Three runs of the `kit` suite: the first fails [`COMMA`], and the last
/// two add `late`. `gone` is in every run.
fn history() -> History {
    let mut h = History::new();
    let base = [
        ("kit", COMMA, false),
        ("kit", BACKSLASH, false),
        ("kit", TAB, false),
        ("kit", UNI, false),
        ("kit", "gone", false),
    ];
    let mut first = base;
    first[0].2 = true;
    h.ingest(&first);
    let mut later = base.to_vec();
    later.push(("kit", "late", false));
    h.ingest(&later);
    h.ingest(&later);
    h
}

/// The tests now: every stored test but `gone`, plus `fresh`, which the
/// store has never seen, and `kit2`'s `late`, a stored name in another
/// suite.
const NOW: &[(&str, &str)] = &[
    ("kit", UNI),
    ("kit", "fresh"),
    ("kit", TAB),
    ("kit", COMMA),
    ("kit", "late"),
    ("kit", BACKSLASH),
    ("kit2", "late"),
];

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).unwrap()
}

type Groups = (Vec<(String, String)>, Vec<(String, String)>);

/// The rich and poor groups of a plan, as `(suite, name)`s.
fn groups(out: &str) -> Groups {
    assert!(out.ends_with('\n') && out.lines().count() == 1, "{out:?}");
    let plan: serde_json::Value = serde_json::from_str(out).unwrap();
    let group = |key: &str| {
        plan[key]
            .as_array()
            .unwrap_or_else(|| panic!("no {key} array: {out}"))
            .iter()
            .map(|t| {
                (
                    t["suite"].as_str().unwrap().to_owned(),
                    t["name"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(plan.as_object().unwrap().len(), 2, "{out}");
    (group("rich"), group("poor"))
}

fn owned(tests: &[(&str, &str)]) -> Vec<(String, String)> {
    tests
        .iter()
        .map(|&(s, n)| (s.to_owned(), n.to_owned()))
        .collect()
}

/// Each test of `now` is in exactly one group, and nothing else is.
fn assert_partition((rich, poor): &Groups, now: &[(&str, &str)]) {
    let all: Vec<&(String, String)> = rich.iter().chain(poor).collect();
    let distinct: BTreeSet<&(String, String)> = all.iter().copied().collect();
    assert_eq!(distinct.len(), all.len(), "a test is planned twice");
    let want = owned(now);
    assert_eq!(distinct, want.iter().collect(), "not the inventory");
}

#[test]
fn plans_the_inventory_from_history() {
    let h = history();
    let out = h.plan(&inventory(NOW), &["--min-runs", "3"]).success();
    let plan = groups(&stdout(&out));
    // COMMA covers the one failing run. `late` has 2 results, under
    // --min-runs, and `fresh` and kit2's `late` have none: all rich, in
    // inventory order. Poor is select's order: same median, so by name.
    assert_eq!(
        plan.0,
        owned(&[
            ("kit", COMMA),
            ("kit", "fresh"),
            ("kit", "late"),
            ("kit2", "late")
        ])
    );
    assert_eq!(
        plan.1,
        owned(&[("kit", BACKSLASH), ("kit", TAB), ("kit", UNI)])
    );
    assert_partition(&plan, NOW);
    assert_eq!(stderr(&out), "");
}

#[test]
fn names_are_written_raw_not_as_select_shows_them() {
    let h = history();
    let select = greenshift()
        .arg("select")
        .arg("--db")
        .arg(&h.db)
        .args(["--min-runs", "3"])
        .assert()
        .success();
    assert!(stdout(&select).contains(r"tab\there"));
    assert!(stdout(&select).contains(r"back\\slash"));
    let out = stdout(&h.plan(&inventory(NOW), &["--min-runs", "3"]).success());
    // The JSON string escapes are JSON's: a tab is \t, a backslash \\.
    assert!(
        out.contains(r#"{"suite":"kit","name":"tab\there"}"#),
        "{out}"
    );
    assert!(
        out.contains(r#"{"suite":"kit","name":"back\\slash/x"}"#),
        "{out}"
    );
    assert!(out.contains(UNI), "{out}");
}

#[test]
fn under_observed_tests_are_rich() {
    // With the default --min-runs 120 no test is poor.
    let h = history();
    let plan = groups(&stdout(&h.plan(&inventory(NOW), &[]).success()));
    assert_eq!(plan.0[0], ("kit".to_owned(), COMMA.to_owned()));
    assert!(plan.1.is_empty(), "{plan:?}");
    assert_partition(&plan, NOW);
}

#[test]
fn with_no_history_every_test_is_rich() {
    let h = History::new();
    let out = h.plan(&inventory(NOW), &[]).success();
    let plan = groups(&stdout(&out));
    assert_eq!(plan.0, owned(NOW));
    assert!(plan.1.is_empty());
    assert_eq!(stderr(&out), "no history: run the full suite\n");
    assert!(!h.db.exists(), "plan created a store");
}

#[test]
fn an_empty_inventory_plans_nothing() {
    let h = history();
    for text in ["", "\n\n"] {
        let out = h.plan(text, &["--min-runs", "3"]).success();
        assert_eq!(stdout(&out), "{\"rich\":[],\"poor\":[]}\n");
    }
}

#[test]
fn the_inventory_can_come_from_stdin() {
    let h = history();
    let out = greenshift()
        .arg("plan")
        .arg("--db")
        .arg(&h.db)
        .args(["--min-runs", "3", "--inventory", "-"])
        .write_stdin(inventory(NOW))
        .assert()
        .success();
    assert_partition(&groups(&stdout(&out)), NOW);
}

/// Three runs of `kit`: `left` fails the first, `right` the second, and
/// `still` never fails, so the cover needs two picks.
fn two_pick_history() -> History {
    let mut h = History::new();
    h.ingest(&[
        ("kit", "left", true),
        ("kit", "right", false),
        ("kit", "still", false),
    ]);
    h.ingest(&[
        ("kit", "left", false),
        ("kit", "right", true),
        ("kit", "still", false),
    ]);
    h.ingest(&[
        ("kit", "left", false),
        ("kit", "right", false),
        ("kit", "still", false),
    ]);
    h
}

const PICKS: &[(&str, &str)] = &[("kit", "still"), ("kit", "right"), ("kit", "left")];

#[test]
fn coverage_ends_rich_early() {
    let h = two_pick_history();
    let whole = groups(&stdout(
        &h.plan(&inventory(PICKS), &["--min-runs", "3"]).success(),
    ));
    // The whole cover is rich, then the rest in inventory order.
    assert_eq!(whole.0.len(), 2, "{whole:?}");
    assert!(!whole.0.contains(&("kit".to_owned(), "still".to_owned())));
    assert_eq!(whole.1, owned(&[("kit", "still")]));
    // Half the failing runs are caught by the first pick alone.
    let out = h
        .plan(&inventory(PICKS), &["--min-runs", "3", "--coverage", "50"])
        .success();
    let half = groups(&stdout(&out));
    assert_eq!(half.0, whole.0[..1], "{half:?}");
    assert_eq!(half.1.len(), 2, "{half:?}");
    assert!(half.1.contains(&whole.0[1]), "{half:?}");
    assert_partition(&half, PICKS);
    assert_eq!(stderr(&out), "");
}

#[test]
fn an_unreached_threshold_is_named_on_stderr() {
    let h = two_pick_history();
    let out = h
        .plan(
            &inventory(PICKS),
            &["--min-runs", "3", "--confidence", "99.9"],
        )
        .success();
    let plan = groups(&stdout(&out));
    assert_eq!(plan.0.len(), 2, "{plan:?}");
    assert_partition(&plan, PICKS);
    let err = stderr(&out);
    assert!(
        err.starts_with("greenshift: confidence 99.9% not reached; the highest is ")
            && err.ends_with("%\n")
            && err.lines().count() == 1,
        "{err}"
    );
}

#[test]
fn a_malformed_inventory_is_an_error() {
    let h = history();
    let good = inventory(&[("kit", "fresh")]);
    for (text, line) in [
        ("not json\n".to_owned(), 1),
        (format!("{good}{{\"suite\":\"kit\"}}\n"), 2),
        (format!("{good}{good}[\"kit\",\"x\"]\n"), 3),
        ("{\"suite\":\"kit\",\"name\":null}\n".to_owned(), 1),
    ] {
        let out = h.plan(&text, &["--min-runs", "3"]).failure().code(1);
        assert_eq!(stdout(&out), "", "{text:?}");
        let err = stderr(&out);
        assert!(
            err.contains(&format!(": not a greenshift inventory: line {line}: ")),
            "{text:?}: {err}"
        );
    }
}

#[test]
fn an_unreadable_inventory_is_an_error() {
    let h = history();
    let out = greenshift()
        .arg("plan")
        .arg("--db")
        .arg(&h.db)
        .arg("--inventory")
        .arg(h.dir.path().join("missing.jsonl"))
        .assert()
        .failure()
        .code(1);
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("inventory "), "{}", stderr(&out));
}

#[test]
fn the_inventory_is_required() {
    let h = history();
    let out = greenshift()
        .arg("plan")
        .arg("--db")
        .arg(&h.db)
        .assert()
        .failure()
        .code(2);
    assert!(
        stderr(&out).contains("--inventory <FILE>"),
        "{}",
        stderr(&out)
    );
    assert_eq!(stdout(&out), "");
}

#[test]
fn a_bad_store_is_an_error() {
    let h = History::new();
    std::fs::write(&h.db, "not sqlite").unwrap();
    let out = h.plan(&inventory(NOW), &[]).failure().code(1);
    assert_eq!(stdout(&out), "");
}

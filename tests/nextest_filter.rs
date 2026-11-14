//! Runs `greenshift nextest-filter` on history ingested from real
//! cargo-nextest runs of a scratch project, and checks with
//! `cargo nextest list -E` which tests its filtersets select.
//!
//! The scratch project's test binaries use a custom harness, so test names
//! can hold characters that mean something in a filterset. Needs
//! cargo-nextest installed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use assert_cmd::Command;

const PACKAGE: &str = "odd-names";

/// The `odd` binary's tests. `FRESH` in the environment adds
/// [`FRESH_TEST`].
const ODD: &[&str] = &[
    "plain::passes",
    "a (b) | c & d, e",
    "x) | all() | test(=y",
    r#"back\slash/and"quote'"#,
    " leading and trailing ",
    "uni::ünï ✓",
    "shared::same",
    "comma,and::colons",
];

/// The `plain` binary's tests. `shared::same` is in both binaries.
const PLAIN: &[&str] = &["shared::same", "plain::fast"];

const FRESH_TEST: &str = "fresh::arrival";

/// A test binary that lists `names` (and [`FRESH_TEST`] for `fresh`, when
/// `FRESH` is set), and fails a test when `FAIL` has a `binary<TAB>name`
/// line for it.
fn harness(binary: &str, names: &[&str], fresh: bool) -> String {
    format!(
        r#"const BINARY: &str = {binary:?};
const NAMES: &[&str] = &{names:?};
const FRESH: Option<&str> = {fresh:?};

fn main() {{
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {{
        if !args.iter().any(|a| a == "--ignored") {{
            let fresh = FRESH.filter(|_| std::env::var_os("FRESH").is_some());
            for name in NAMES.iter().copied().chain(fresh) {{
                println!("{{name}}: test");
            }}
        }}
        return;
    }}
    let name = args.iter().find(|a| !a.starts_with("--")).expect("a test name");
    let fail = std::env::var("FAIL").unwrap_or_default();
    if fail.lines().any(|line| line == format!("{{BINARY}}\t{{name}}")) {{
        eprintln!("failing {{name}} as asked");
        std::process::exit(101);
    }}
}}
"#,
        fresh = fresh.then_some(FRESH_TEST),
    )
}

struct Project {
    dir: tempfile::TempDir,
    db: PathBuf,
    runs: usize,
}

impl Project {
    fn new() -> Project {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::create_dir_all(root.join(".config")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{PACKAGE}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\
                 publish = false\n\n[workspace]\n\n[lib]\ntest = false\ndoctest = false\n\n\
                 [[test]]\nname = \"odd\"\nharness = false\n\n\
                 [[test]]\nname = \"plain\"\nharness = false\n"
            ),
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("tests/odd.rs"), harness("odd", ODD, true)).unwrap();
        std::fs::write(root.join("tests/plain.rs"), harness("plain", PLAIN, false)).unwrap();
        std::fs::write(
            root.join(".config/nextest.toml"),
            "[profile.default.junit]\npath = \"junit.xml\"\n",
        )
        .unwrap();
        let db = dir.path().join("history.sqlite");
        Project { dir, db, runs: 0 }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// `cargo nextest <args>` in the project, away from the nextest or
    /// cargo run this test is part of.
    fn cargo_nextest(&self, args: &[&str]) -> Process {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Process::new(cargo);
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy().into_owned();
            let inherited =
                key.starts_with("NEXTEST_") || (key.starts_with("CARGO_") && key != "CARGO_HOME");
            if inherited {
                cmd.env_remove(&key);
            }
        }
        cmd.current_dir(self.root()).arg("nextest").args(args);
        cmd
    }

    /// Run every test with nextest, failing `fail` (`(binary, name)`), and
    /// ingest its JUnit as one run.
    fn run(&mut self, fail: &[(&str, &str)]) {
        let fail: String = fail.iter().map(|(b, n)| format!("{b}\t{n}\n")).collect();
        let out = self
            .cargo_nextest(&["run", "--no-fail-fast"])
            .env("FAIL", &fail)
            .env_remove("FRESH")
            .output()
            .expect("cargo nextest runs (is cargo-nextest installed?)");
        let junit = self.root().join("target/nextest/default/junit.xml");
        assert!(
            junit.exists(),
            "nextest wrote no JUnit: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        self.runs += 1;
        let kept = self.dir.path().join(format!("run{}.xml", self.runs));
        std::fs::rename(&junit, &kept).unwrap();
        greenshift()
            .arg("ingest")
            .arg("--db")
            .arg(&self.db)
            .args(["--source", "nextest"])
            .arg(&kept)
            .assert()
            .success();
    }

    fn filter(&self, args: &[&str]) -> String {
        let out = greenshift()
            .arg("nextest-filter")
            .arg("--db")
            .arg(&self.db)
            .args(args)
            .assert()
            .success();
        let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
        let expr = stdout
            .strip_suffix('\n')
            .expect("the filterset ends its line");
        assert!(!expr.contains('\n'), "more than one line: {stdout:?}");
        assert!(!expr.is_empty(), "an empty filterset");
        expr.to_owned()
    }

    /// The tests `cargo nextest list -E expr` selects, as `(binary, name)`,
    /// with [`FRESH_TEST`] in the inventory.
    fn list(&self, expr: &str) -> BTreeSet<(String, String)> {
        let out = self
            .cargo_nextest(&["list", "--message-format", "oneline", "-E", expr])
            .env("FRESH", "1")
            .output()
            .expect("cargo nextest runs (is cargo-nextest installed?)");
        assert!(
            out.status.success(),
            "nextest refused {expr:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let prefix = format!("{PACKAGE}::");
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|line| {
                let (binary, name) = line.split_once(' ').expect("binary and name");
                let binary = binary.strip_prefix(&prefix).expect("a test binary");
                (binary.to_owned(), name.to_owned())
            })
            .collect()
    }
}

fn greenshift() -> Command {
    Command::cargo_bin("greenshift").expect("binary builds")
}

fn tests(binary: &str, names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .map(|n| (binary.to_owned(), (*n).to_owned()))
        .collect()
}

fn set(groups: &[Vec<(String, String)>]) -> BTreeSet<(String, String)> {
    groups.iter().flatten().cloned().collect()
}

#[test]
fn filtersets_select_rich_and_poor_tests_in_nextest() {
    let mut p = Project::new();
    // One row per run of the first three, each its own failing test, so
    // all three are picked; the fourth run is green.
    let rich = [
        ("odd", "a (b) | c & d, e"),
        ("odd", r#"back\slash/and"quote'"#),
        ("plain", "shared::same"),
    ];
    for fail in &rich {
        p.run(&[*fail]);
    }
    p.run(&[]);

    let poor = set(&[
        tests(
            "odd",
            &[
                "plain::passes",
                "x) | all() | test(=y",
                " leading and trailing ",
                "uni::ünï ✓",
                "shared::same",
                "comma,and::colons",
            ],
        ),
        tests("plain", &["plain::fast"]),
    ]);
    let mut rich_and_fresh = set(&[tests("odd", &[FRESH_TEST])]);
    rich_and_fresh.extend(rich.iter().map(|(b, n)| ((*b).to_owned(), (*n).to_owned())));

    let rich_expr = p.filter(&["--source", "nextest", "--min-runs", "1", "--rich"]);
    assert!(rich_expr.starts_with("not ("), "{rich_expr}");
    assert_eq!(p.list(&rich_expr), rich_and_fresh, "{rich_expr}");
    assert_eq!(
        p.filter(&["--source", "nextest", "--min-runs", "1"]),
        rich_expr,
        "--rich is the default"
    );

    let poor_expr = p.filter(&["--source", "nextest", "--min-runs", "1", "--poor"]);
    assert_eq!(p.list(&poor_expr), poor, "{poor_expr}");
}

#[test]
fn empty_selections_match_nothing_or_everything() {
    let mut p = Project::new();
    p.run(&[]);
    // One green run: no rich test, and with the default --min-runs every
    // test is new, so none is poor either.
    let poor_expr = p.filter(&["--poor"]);
    assert_eq!(poor_expr, "none()");
    assert_eq!(p.list(&poor_expr), BTreeSet::new());

    let rich_expr = p.filter(&["--rich"]);
    assert_eq!(rich_expr, "all()");
    let everything = set(&[
        tests("odd", ODD),
        tests("odd", &[FRESH_TEST]),
        tests("plain", PLAIN),
    ]);
    assert_eq!(p.list(&rich_expr), everything);
}

#[test]
fn rich_and_poor_are_exclusive() {
    greenshift()
        .args(["nextest-filter", "--rich", "--poor"])
        .assert()
        .failure()
        .code(2)
        .stdout("")
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn no_history_prints_nothing_and_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("missing.sqlite");
    for class in ["--rich", "--poor"] {
        greenshift()
            .arg("nextest-filter")
            .arg("--db")
            .arg(&db)
            .arg(class)
            .assert()
            .code(2)
            .stdout("")
            .stderr("no history: run the full suite\n");
    }
    assert!(!Path::new(&db).exists());
}

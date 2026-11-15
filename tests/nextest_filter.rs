//! Runs `greenshift nextest-filter` on history ingested from real
//! cargo-nextest runs of a scratch project, and checks with
//! `cargo nextest list -E` which tests its filtersets select.
//!
//! The scratch project's test binaries use a custom harness, so test names
//! can hold characters that mean something in a filterset. Needs
//! cargo-nextest installed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Output};

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

/// The `gone` binary's tests; it is in the project only while
/// [`Project::with_gone`] says so.
const GONE: &[&str] = &["gone::flaky", "gone::steady"];

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

/// `fail`'s `(binary, name)`s as the harness reads `FAIL`.
fn fail_lines(fail: &[(&str, &str)]) -> String {
    fail.iter().map(|(b, n)| format!("{b}\t{n}\n")).collect()
}

/// The two filtersets of one [`Project::run_passes`] run.
#[derive(Debug, PartialEq)]
struct Passes {
    poor: String,
    rich: String,
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
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("tests/odd.rs"), harness("odd", ODD, true)).unwrap();
        std::fs::write(root.join("tests/plain.rs"), harness("plain", PLAIN, false)).unwrap();
        std::fs::write(root.join("tests/gone.rs"), harness("gone", GONE, false)).unwrap();
        // greenshift's own nextest config, with CI's two-pass profiles.
        std::fs::write(
            root.join(".config/nextest.toml"),
            include_str!("../.config/nextest.toml"),
        )
        .unwrap();
        let db = dir.path().join("history.sqlite");
        let project = Project { dir, db, runs: 0 };
        project.with_gone(false);
        project
    }

    /// Write the manifest, with the `gone` test binary or without it.
    fn with_gone(&self, gone: bool) {
        let mut manifest = format!(
            "[package]\nname = \"{PACKAGE}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\
             publish = false\nautotests = false\n\n[workspace]\n\n\
             [lib]\ntest = false\ndoctest = false\n"
        );
        let binaries: &[&str] = if gone {
            &["odd", "plain", "gone"]
        } else {
            &["odd", "plain"]
        };
        for binary in binaries {
            manifest.push_str(&format!(
                "\n[[test]]\nname = \"{binary}\"\nharness = false\n"
            ));
        }
        std::fs::write(self.root().join("Cargo.toml"), manifest).unwrap();
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
        self.runs += 1;
        let kept = self.dir.path().join(format!("run{}.xml", self.runs));
        self.pass("default", &["--no-fail-fast"], &fail_lines(fail), &kept);
        self.ingest(&[kept]);
    }

    /// One run as CI makes it (see `.github/workflows/ci.yml`): the rich
    /// and then the poor filterset from the history so far, with
    /// `--min-runs 3`, each run by nextest as a pass of its own under CI's
    /// profile for it (`ci-rich`, `ci-poor`), failing `fail`, and both
    /// passes' JUnit ingested together as one run. Returns both
    /// filtersets. Keep `--min-runs` in step with the workflow's.
    fn run_passes(&mut self, fail: &[(&str, &str)]) -> Passes {
        let [rich, poor] = ["--rich", "--poor"]
            .map(|class| self.filter(&["--source", "nextest", "--min-runs", "3", class]));
        let fail = fail_lines(fail);
        self.runs += 1;
        let mut kept = Vec::new();
        for (profile, expr) in [("ci-rich", &rich), ("ci-poor", &poor)] {
            let file = self
                .dir
                .path()
                .join(format!("run{}-{profile}.xml", self.runs));
            // Either pass can be empty, which nextest fails by default.
            // One test at a time, so a profile that stopped at the first
            // failure would leave the tests after it out of the run. Only
            // the poor pass has a failure here, so this checks ci-poor's
            // fail-fast, not ci-rich's.
            let out = self.pass(
                profile,
                &["--no-tests=pass", "--test-threads", "1", "-E", expr],
                &fail,
                &file,
            );
            // Green, or failing only a test asked to fail (nextest's exit
            // code 100); never refusing to run, say an empty pass (4).
            let failed_as_asked = !fail.is_empty() && out.status.code() == Some(100);
            assert!(
                out.status.success() || failed_as_asked,
                "nextest exited {:?} for {expr:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
            kept.push(file);
        }
        self.ingest(&kept);
        Passes { poor, rich }
    }

    /// `cargo nextest run --profile <profile> <args>`, failing the tests
    /// `fail` lists, with its JUnit moved to `kept`.
    fn pass(&self, profile: &str, args: &[&str], fail: &str, kept: &Path) -> Output {
        let out = self
            .cargo_nextest(&["run", "--profile", profile])
            .args(args)
            .env("FAIL", fail)
            .env_remove("FRESH")
            .output()
            .expect("cargo nextest runs (is cargo-nextest installed?)");
        let junit = self
            .root()
            .join(format!("target/nextest/{profile}/junit.xml"));
        assert!(
            junit.exists(),
            "nextest wrote no JUnit for {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::rename(&junit, kept).unwrap();
        out
    }

    /// Ingest `files` into the store as one run.
    fn ingest(&self, files: &[PathBuf]) {
        greenshift()
            .arg("ingest")
            .arg("--db")
            .arg(&self.db)
            .args(["--source", "nextest"])
            .args(files)
            .assert()
            .success();
    }

    /// The project's current `cargo nextest list --list-type binaries-only
    /// --message-format json`, in a file.
    fn inventory(&self) -> PathBuf {
        let out = self
            .cargo_nextest(&[
                "list",
                "--list-type",
                "binaries-only",
                "--message-format",
                "json",
            ])
            .output()
            .expect("cargo nextest runs (is cargo-nextest installed?)");
        assert!(
            out.status.success(),
            "nextest could not list: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let file = self.dir.path().join("inventory.json");
        std::fs::write(&file, out.stdout).unwrap();
        file
    }

    /// `greenshift nextest-filter` with `args` and the current inventory.
    fn filter(&self, args: &[&str]) -> String {
        let out = greenshift()
            .arg("nextest-filter")
            .arg("--db")
            .arg(&self.db)
            .arg("--inventory")
            .arg(self.inventory())
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
fn a_binary_gone_from_the_inventory_is_left_out() {
    let mut p = Project::new();
    p.with_gone(true);
    // gone::flaky and plain::passes each fail in a run of their own, so
    // both are rich; every other test is poor.
    p.run(&[("gone", "gone::flaky")]);
    p.run(&[("odd", "plain::passes")]);
    p.run(&[]);
    p.with_gone(false);

    // Kept in, binary_id(=odd-names::gone) would make nextest refuse the
    // whole filterset.
    let rich_expr = p.filter(&["--source", "nextest", "--min-runs", "1", "--rich"]);
    assert!(!rich_expr.contains("gone"), "{rich_expr}");
    assert_eq!(
        p.list(&rich_expr),
        set(&[tests("odd", &["plain::passes", FRESH_TEST])]),
        "{rich_expr}"
    );

    let poor_expr = p.filter(&["--source", "nextest", "--min-runs", "1", "--poor"]);
    assert!(!poor_expr.contains("gone"), "{poor_expr}");
    let mut poor = set(&[tests("odd", ODD), tests("plain", PLAIN)]);
    poor.remove(&("odd".to_owned(), "plain::passes".to_owned()));
    assert_eq!(p.list(&poor_expr), poor, "{poor_expr}");
}

#[test]
fn the_inventory_can_come_from_stdin() {
    let mut p = Project::new();
    p.run(&[]);
    let inventory = std::fs::read(p.inventory()).unwrap();
    greenshift()
        .arg("nextest-filter")
        .arg("--db")
        .arg(&p.db)
        .args(["--inventory", "-", "--poor"])
        .write_stdin(inventory)
        .assert()
        .success()
        .stdout("none()\n");
}

/// A `--inventory` file listing no binaries, in `dir`.
fn empty_inventory(dir: &Path) -> PathBuf {
    let file = dir.join("inventory.json");
    std::fs::write(&file, r#"{"rust-build-meta":{},"rust-binaries":{}}"#).unwrap();
    file
}

#[test]
fn an_inventory_is_required_and_must_be_a_nextest_listing() {
    let dir = tempfile::tempdir().unwrap();
    greenshift()
        .args(["nextest-filter", "--poor"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicates::str::contains("--inventory"));
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "[]").unwrap();
    greenshift()
        .arg("nextest-filter")
        .arg("--inventory")
        .arg(&bad)
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicates::str::contains("not cargo nextest list JSON"));
    greenshift()
        .arg("nextest-filter")
        .arg("--inventory")
        .arg(dir.path().join("missing.json"))
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn rich_and_poor_are_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    greenshift()
        .args(["nextest-filter", "--rich", "--poor"])
        .arg("--inventory")
        .arg(empty_inventory(dir.path()))
        .assert()
        .failure()
        .code(2)
        .stdout("")
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn no_history_matches_every_test_as_rich() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("missing.sqlite");
    for (class, expr) in [("--rich", "all()\n"), ("--poor", "none()\n")] {
        greenshift()
            .arg("nextest-filter")
            .arg("--db")
            .arg(&db)
            .arg("--inventory")
            .arg(empty_inventory(dir.path()))
            .arg(class)
            .assert()
            .success()
            .stdout(expr)
            .stderr("no history: run the full suite\n");
    }
    assert!(!Path::new(&db).exists());
}

#[test]
fn a_fresh_project_runs_everything() {
    let mut p = Project::new();
    // No store yet.
    let everything = set(&[
        tests("odd", ODD),
        tests("odd", &[FRESH_TEST]),
        tests("plain", PLAIN),
    ]);
    assert_eq!(p.filter(&["--rich"]), "all()");
    assert_eq!(p.list(&p.filter(&[])), everything);
    assert_eq!(p.filter(&["--poor"]), "none()");
    // A store with runs only from another source is no history either.
    p.run(&[]);
    assert_eq!(p.filter(&["--source", "elsewhere", "--rich"]), "all()");
    assert_eq!(p.filter(&["--source", "elsewhere", "--poor"]), "none()");
}

#[test]
fn a_failure_in_one_two_pass_run_is_rich_in_the_next() {
    let mut p = Project::new();
    let everything = set(&[
        tests("odd", ODD),
        tests("odd", &[FRESH_TEST]),
        tests("plain", PLAIN),
    ]);
    // With fewer than 3 runs on record every test is new, so the poor pass
    // is empty and every test runs in the rich one.
    for _ in 0..3 {
        let passes = Passes {
            poor: "none()".to_owned(),
            rich: "all()".to_owned(),
        };
        assert_eq!(p.run_passes(&[]), passes);
    }

    // Three green runs: every test on record is poor. One of them fails
    // in this run's poor pass. Tests run after it there (one at a time, by
    // name), so a poor pass that stopped at it would be cut short.
    let failing = ("odd", "uni::ünï ✓");
    let passes = p.run_passes(&[failing]);
    let mut poor = set(&[tests("odd", ODD), tests("plain", PLAIN)]);
    assert_eq!(p.list(&passes.poor), poor, "{passes:?}");
    assert_eq!(
        p.list(&passes.rich),
        set(&[tests("odd", &[FRESH_TEST])]),
        "{passes:?}"
    );

    // The next run reads that failure back: the test that failed is rich
    // now, and the two passes still split every test between them.
    let passes = p.run_passes(&[]);
    let failing = (failing.0.to_owned(), failing.1.to_owned());
    poor.remove(&failing);
    let rich = set(&[tests("odd", &[FRESH_TEST]), vec![failing]]);
    assert_eq!(p.list(&passes.poor), poor, "{passes:?}");
    assert_eq!(p.list(&passes.rich), rich, "{passes:?}");
    assert!(poor.is_disjoint(&rich));
    assert_eq!(&poor | &rich, everything);
}

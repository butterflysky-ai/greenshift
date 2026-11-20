//! `greenshift init nextest` on scratch repositories: what it writes,
//! that a second run changes nothing, `--force`, and what it refuses.
//!
//! The generated workflows are compared with golden files in
//! `tests/golden/`. After a deliberate change to a template, rewrite them
//! with `GREENSHIFT_BLESS=1 cargo nextest run --test init` and review the
//! diff.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

/// A release and checksum for tests only: the checksum is made up and
/// matches no real binary.
const RELEASE: &str = "v0.2.0";
const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const GITHUB_WORKFLOW: &str = ".github/workflows/greenshift.yml";

fn greenshift(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("greenshift").expect("binary builds");
    cmd.current_dir(dir);
    cmd
}

/// `greenshift init nextest` in `dir` with the test release and checksum,
/// then `args`.
fn init(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = greenshift(dir);
    cmd.args(["init", "nextest", "--release", RELEASE, "--sha256", SHA256])
        .args(args);
    cmd
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// `written` matches the golden file `name`, or with `GREENSHIFT_BLESS`
/// set, becomes it.
fn assert_golden(written: &Path, name: &str) {
    let actual = std::fs::read_to_string(written).unwrap();
    let path = golden(name);
    if std::env::var_os("GREENSHIFT_BLESS").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    assert!(
        actual == expected,
        "{} differs from {}; if the change is meant, rerun with GREENSHIFT_BLESS=1\n\
         --- actual ---\n{actual}",
        written.display(),
        path.display()
    );
}

/// Every file under `dir`, with its content, sorted by path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_owned();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn stdout_of(cmd: &mut Command) -> String {
    let out = cmd.assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn github_writes_the_golden_workflow_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let out = stdout_of(&mut init(dir.path(), &["--ci", "github"]));
    assert!(out.contains(&format!("wrote {GITHUB_WORKFLOW}")), "{out}");
    assert_golden(&dir.path().join(GITHUB_WORKFLOW), "init-nextest-github.yml");
}

#[test]
fn the_forge_is_found_from_the_repository_when_not_given() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".github")).unwrap();
    let out = stdout_of(&mut init(dir.path(), &[]));
    assert!(out.contains("github"), "{out}");
    assert!(dir.path().join(GITHUB_WORKFLOW).is_file());
}

#[test]
fn with_no_forge_to_go_on_it_asks_for_one_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &[])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--ci"));
    assert!(snapshot(dir.path()).is_empty());
    std::fs::create_dir(dir.path().join(".github")).unwrap();
    std::fs::create_dir(dir.path().join(".forgejo")).unwrap();
    init(dir.path(), &[])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--ci"));
    assert!(snapshot(dir.path()).is_empty());
}

#[test]
fn a_second_run_changes_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &["--ci", "github"]).assert().success();
    let before = snapshot(dir.path());
    let out = stdout_of(&mut init(dir.path(), &["--ci", "github"]));
    assert_eq!(snapshot(dir.path()), before);
    assert!(out.contains("nothing to change"), "{out}");
    assert!(!out.contains("wrote"), "{out}");
}

#[test]
fn an_existing_different_workflow_is_left_alone_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let workflow = dir.path().join(GITHUB_WORKFLOW);
    std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    std::fs::write(&workflow, "# mine\n").unwrap();
    let before = snapshot(dir.path());
    init(dir.path(), &["--ci", "github"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(GITHUB_WORKFLOW))
        .stderr(predicate::str::contains("--force"));
    assert_eq!(snapshot(dir.path()), before, "nothing is written");
}

#[test]
fn force_overwrites_the_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let workflow = dir.path().join(GITHUB_WORKFLOW);
    std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    std::fs::write(&workflow, "# mine\n").unwrap();
    let out = stdout_of(&mut init(dir.path(), &["--ci", "github", "--force"]));
    assert!(
        out.contains(&format!("overwrote {GITHUB_WORKFLOW}")),
        "{out}"
    );
    assert_golden(&workflow, "init-nextest-github.yml");
}

#[test]
fn a_new_pin_needs_force_and_then_replaces_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &["--ci", "github"]).assert().success();
    let other = "f".repeat(64);
    let mut repin = greenshift(dir.path());
    repin.args([
        "init",
        "nextest",
        "--ci",
        "github",
        "--release",
        "v0.3.1",
        "--sha256",
        &other,
    ]);
    repin.assert().code(1);
    repin.arg("--force").assert().success();
    let text = std::fs::read_to_string(dir.path().join(GITHUB_WORKFLOW)).unwrap();
    assert!(text.contains("v0.3.1") && text.contains(&other));
    assert!(!text.contains(SHA256) && !text.contains(RELEASE));
}

#[test]
fn a_missing_or_invalid_release_or_checksum_is_refused() {
    let short = "0".repeat(63);
    let long = "0".repeat(65);
    let not_hex = "g".repeat(64);
    let cases: Vec<Vec<&str>> = vec![
        vec!["--sha256", SHA256],
        vec!["--release", RELEASE],
        vec!["--release", "", "--sha256", SHA256],
        vec!["--release", "0.2.0", "--sha256", SHA256],
        vec!["--release", "v0.2", "--sha256", SHA256],
        vec!["--release", "v0.2.0-rc.1", "--sha256", SHA256],
        vec!["--release", "v00.2.0", "--sha256", SHA256],
        vec!["--release", "latest", "--sha256", SHA256],
        vec!["--release", "v0.2.0\n", "--sha256", SHA256],
        // v0.1.0 predates `run nextest` and durable history.
        vec!["--release", "v0.1.0", "--sha256", SHA256],
        vec!["--release", "v0.0.9", "--sha256", SHA256],
        vec!["--release", RELEASE, "--sha256", ""],
        vec!["--release", RELEASE, "--sha256", &short],
        vec!["--release", RELEASE, "--sha256", &long],
        vec!["--release", RELEASE, "--sha256", &not_hex],
    ];
    for args in cases {
        let dir = tempfile::tempdir().unwrap();
        greenshift(dir.path())
            .args(["init", "nextest", "--ci", "github"])
            .args(&args)
            .assert()
            .code(2);
        assert!(snapshot(dir.path()).is_empty(), "{args:?} wrote something");
    }
}

#[test]
fn an_upper_case_checksum_is_written_in_lower_case() {
    let dir = tempfile::tempdir().unwrap();
    greenshift(dir.path())
        .args(["init", "nextest", "--ci", "github", "--release", RELEASE])
        .args(["--sha256", &SHA256.to_uppercase()])
        .assert()
        .success();
    assert_golden(&dir.path().join(GITHUB_WORKFLOW), "init-nextest-github.yml");
}

#[test]
fn usage_errors_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["init"][..],
        &["init", "lein"],
        &["init", "nextest", "--ci", "gitlab"],
        &["init", "nextest", "--ci", "github", "--runs-on", "a b"],
        &["init", "nextest", "--ci", "github", "--runs-on", ""],
        &["init", "nextest", "--ci", "github", "--runs-on", "x\ny"],
    ] {
        greenshift(dir.path())
            .args(args)
            .args(["--release", RELEASE, "--sha256", SHA256])
            .assert()
            .code(2);
    }
    assert!(snapshot(dir.path()).is_empty());
}

#[test]
fn runs_on_names_the_runner() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &["--ci", "github", "--runs-on", "ubuntu-24.04"])
        .assert()
        .success();
    let text = std::fs::read_to_string(dir.path().join(GITHUB_WORKFLOW)).unwrap();
    assert_eq!(text.matches("runs-on: ubuntu-24.04").count(), 2, "{text}");
    assert!(!text.contains("ubuntu-latest"));
}

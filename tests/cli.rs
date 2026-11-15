use assert_cmd::Command;
use predicates::prelude::*;

fn greenshift() -> Command {
    Command::cargo_bin("greenshift").expect("binary builds")
}

#[test]
fn version_prints_crate_version() {
    greenshift()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_describes_the_tool() {
    greenshift()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: greenshift"));
}

/// A name holding a terminal escape and a newline, and how greenshift
/// shows it.
#[cfg(unix)]
const EVIL: &str = "\u{1b}[31mred\nline";
#[cfg(unix)]
const EVIL_SHOWN: &str = r"\u{1b}[31mred\nline";

#[cfg(unix)]
fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

/// The output holds `shown` and no raw ESC or `raw`.
#[cfg(unix)]
fn shows_escaped(output: &[u8], shown: &str) {
    let text = String::from_utf8_lossy(output);
    assert!(text.contains(shown), "{text:?}");
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(!text.contains(EVIL), "{text:?}");
}

// Control characters are not allowed in Windows file names.
#[cfg(unix)]
#[test]
fn a_junit_path_from_argv_is_shown_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let out = greenshift()
        .arg("ingest")
        .arg("--db")
        .arg(dir.path().join("history.sqlite"))
        .arg(dir.path().join(EVIL))
        .assert()
        .failure()
        .code(1);
    shows_escaped(
        &out.get_output().stderr,
        &format!("{EVIL_SHOWN}: could not read file"),
    );
}

#[cfg(unix)]
#[test]
fn a_db_path_from_argv_is_shown_escaped_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let out = greenshift()
        .arg("ingest")
        .arg("--db")
        .arg(dir.path().join(EVIL))
        .arg(fixture("nextest.xml"))
        .assert()
        .success();
    shows_escaped(&out.get_output().stdout, &format!("{EVIL_SHOWN}\n"));
}

#[cfg(unix)]
#[test]
fn a_db_path_from_argv_is_shown_escaped_in_store_errors() {
    let dir = tempfile::tempdir().unwrap();
    // A regular file where the store's directory would go.
    std::fs::write(dir.path().join(EVIL), "").unwrap();
    let out = greenshift()
        .arg("ingest")
        .arg("--db")
        .arg(dir.path().join(EVIL).join("history.sqlite"))
        .arg(fixture("nextest.xml"))
        .assert()
        .failure()
        .code(1);
    shows_escaped(
        &out.get_output().stderr,
        &format!("could not create {}", dir.path().join(EVIL_SHOWN).display()),
    );

    let db = dir.path().join(format!("{EVIL}.sqlite"));
    std::fs::write(
        &db,
        "not sqlite at all, but long enough to be read as a header....",
    )
    .unwrap();
    for args in [&["select"][..], &["nextest-filter", "--inventory", "-"]] {
        let out = greenshift()
            .args(args)
            .arg("--db")
            .arg(&db)
            .write_stdin(r#"{"rust-build-meta":{},"rust-suites":{}}"#)
            .assert()
            .failure()
            .code(1);
        shows_escaped(&out.get_output().stderr, &format!("{EVIL_SHOWN}.sqlite"));
    }
}

#[cfg(unix)]
#[test]
fn an_inventory_path_from_argv_is_shown_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let out = greenshift()
        .args(["nextest-filter", "--inventory"])
        .arg(dir.path().join(EVIL))
        .assert()
        .failure()
        .code(1);
    shows_escaped(
        &out.get_output().stderr,
        &format!("inventory {}", dir.path().join(EVIL_SHOWN).display()),
    );
}

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

/// A SQLite file that `select` (read only) and `ingest` (read-write) both
/// refuse, in each way that names the store: another program's database,
/// one from a newer greenshift, and one with a negative schema version.
#[cfg(unix)]
#[test]
fn a_db_path_from_argv_is_shown_escaped_when_the_store_is_refused() {
    use greenshift::store::{APPLICATION_ID, SCHEMA_VERSION};
    let cases = [
        (0, 0, "is not a greenshift history store".to_owned()),
        (
            APPLICATION_ID,
            SCHEMA_VERSION + 1,
            format!("has schema version {}, newer", SCHEMA_VERSION + 1),
        ),
        (
            APPLICATION_ID,
            -1,
            "has schema version -1, which no greenshift writes".to_owned(),
        ),
    ];
    for (application_id, user_version, refusal) in cases {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{EVIL}.sqlite"));
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("CREATE TABLE theirs (x)", []).unwrap();
        conn.pragma_update(None, "application_id", application_id)
            .unwrap();
        conn.pragma_update(None, "user_version", user_version)
            .unwrap();
        drop(conn);
        let shown = format!("{}.sqlite {refusal}", dir.path().join(EVIL_SHOWN).display());
        let mut select = greenshift();
        select.args(["select", "--db"]).arg(&db);
        let mut ingest = greenshift();
        ingest
            .args(["ingest", "--db"])
            .arg(&db)
            .arg(fixture("nextest.xml"));
        for mut cmd in [select, ingest] {
            let out = cmd.assert().failure().code(1);
            shows_escaped(&out.get_output().stderr, &shown);
        }
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

#[cfg(unix)]
#[test]
fn an_inventory_that_is_not_a_listing_is_shown_escaped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(EVIL), "not json").unwrap();
    let out = greenshift()
        .args(["nextest-filter", "--inventory"])
        .arg(dir.path().join(EVIL))
        .assert()
        .failure()
        .code(1);
    shows_escaped(
        &out.get_output().stderr,
        &format!(
            "inventory {}: not cargo nextest list JSON",
            dir.path().join(EVIL_SHOWN).display()
        ),
    );
}

// Bytes that are not UTF-8 show as U+FFFD, as Path::display shows them.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_shown_with_replacement_characters() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir
        .path()
        .join(std::ffi::OsStr::from_bytes(b"bad\xffname.xml"));
    let out = greenshift()
        .arg("ingest")
        .arg("--db")
        .arg(dir.path().join("history.sqlite"))
        .arg(&file)
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("bad\u{fffd}name.xml: could not read file"),
        "{stderr:?}"
    );
}

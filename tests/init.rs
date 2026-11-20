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
    assert!(
        out.contains("found .github, so writing for github"),
        "{out}"
    );
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
    assert!(
        out.contains(&format!("{GITHUB_WORKFLOW} is already up to date")),
        "{out}"
    );
    assert!(!out.contains("wrote"), "{out}");
}

#[test]
fn force_rewrites_nothing_that_is_already_as_wanted() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &["--ci", "github"]).assert().success();
    let before = snapshot(dir.path());
    let out = stdout_of(&mut init(dir.path(), &["--ci", "github", "--force"]));
    assert_eq!(snapshot(dir.path()), before);
    assert!(
        out.contains(&format!("{GITHUB_WORKFLOW} is already up to date")),
        "{out}"
    );
    assert!(!out.contains("overwrote"), "{out}");
    assert!(out.contains("nothing to change"), "{out}");
}

/// The mode a file newly made in `dir` gets from the process's umask.
#[cfg(unix)]
fn fresh_mode(dir: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    let probe = dir.join("probe");
    std::fs::File::create(&probe).unwrap();
    let mode = std::fs::metadata(&probe).unwrap().permissions().mode() & 0o777;
    std::fs::remove_file(probe).unwrap();
    mode
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn written_files_get_ordinary_modes_and_rewritten_ones_keep_theirs() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let fresh = fresh_mode(dir.path());
    write(dir.path(), NEXTEST_TOML, "[profile.ci]\nretries = 1\n");
    let toml = dir.path().join(NEXTEST_TOML);
    std::fs::set_permissions(&toml, std::fs::Permissions::from_mode(0o664)).unwrap();
    init(dir.path(), &["--ci", "github"]).assert().success();
    assert_eq!(junit_path(dir.path()).as_deref(), Some("junit.xml"));
    assert_eq!(mode(&toml), 0o664, "the rewritten file keeps its mode");
    assert_eq!(
        mode(&dir.path().join(GITHUB_WORKFLOW)),
        fresh,
        "a new file gets the mode any new file would"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_on_the_way_to_a_file_is_never_followed_or_replaced() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("credentials.toml");
    std::fs::write(&secret, "[registry]\ntoken = \"secret\"\n").unwrap();
    // A symlinked file, and a symlinked directory on the way to one.
    for (link, target, other) in [
        (NEXTEST_TOML, secret.clone(), GITHUB_WORKFLOW),
        (".github", outside.path().to_owned(), NEXTEST_TOML),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join(link);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, &at).unwrap();
        let outside_before = snapshot(outside.path());
        for force in [false, true] {
            let mut cmd = init(dir.path(), &["--ci", "github"]);
            if force {
                cmd.arg("--force");
            }
            cmd.assert()
                .code(1)
                .stderr(predicate::str::contains(link))
                .stderr(predicate::str::contains("symlink"));
            let meta = std::fs::symlink_metadata(&at).unwrap();
            assert!(meta.file_type().is_symlink(), "{link}: link kept");
            assert_eq!(snapshot(outside.path()), outside_before, "{link}");
            assert!(
                !dir.path().join(other).exists(),
                "{link}: nothing is written"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_written_means_none_is_written() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".config");
    std::fs::create_dir(&config).unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(config.join("probe")).is_ok() {
        // Running as root, which can write anywhere: nothing to test.
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let out = init(dir.path(), &["--ci", "github"]).assert().code(1);
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(stderr.contains(NEXTEST_TOML), "{stderr}");
    assert!(stderr.contains("nothing written"), "{stderr}");
    assert!(
        !dir.path().join(GITHUB_WORKFLOW).exists(),
        "the workflow is not written without the nextest.toml"
    );
    assert!(snapshot(dir.path()).is_empty());
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
    assert_eq!(text.matches("runs-on: 'ubuntu-24.04'").count(), 2, "{text}");
    assert!(!text.contains("ubuntu-latest"));
}

const NEXTEST_TOML: &str = ".config/nextest.toml";

/// The JUnit path `.config/nextest.toml` in `dir` gives the default
/// profile, as nextest would read it.
fn junit_path(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(NEXTEST_TOML)).unwrap();
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    doc.get("profile")?
        .get("default")?
        .get("junit")?
        .get("path")?
        .as_str()
        .map(str::to_owned)
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_missing_nextest_toml_is_written_with_the_junit_stanza() {
    let dir = tempfile::tempdir().unwrap();
    let out = stdout_of(&mut init(dir.path(), &["--ci", "github"]));
    assert!(out.contains(&format!("wrote {NEXTEST_TOML}")), "{out}");
    assert_eq!(junit_path(dir.path()).as_deref(), Some("junit.xml"));
    let text = std::fs::read_to_string(dir.path().join(NEXTEST_TOML)).unwrap();
    assert!(text.starts_with(STANZA_COMMENT), "{text}");
}

/// The comment `init` puts above the stanza it adds.
const STANZA_COMMENT: &str = "# JUnit output, for `greenshift ingest` and other JUnit readers.\n\
                              # Added by `greenshift init nextest`.\n";

#[test]
fn an_existing_junit_stanza_is_left_alone_even_with_force() {
    for force in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mine = "# mine\n[profile.default.junit]\npath = \"results/junit.xml\"\n";
        write(dir.path(), NEXTEST_TOML, mine);
        let mut cmd = init(dir.path(), &["--ci", "github"]);
        if force {
            cmd.arg("--force");
        }
        let out = stdout_of(&mut cmd);
        assert!(
            out.contains(&format!("{NEXTEST_TOML} already sets")),
            "{out}"
        );
        let text = std::fs::read_to_string(dir.path().join(NEXTEST_TOML)).unwrap();
        assert_eq!(text, mine, "force={force}");
    }
}

#[test]
fn the_stanza_is_added_to_a_nextest_toml_without_one_keeping_the_rest() {
    let cases = [
        "# Our nextest settings.\n[profile.ci]\nfail-fast = false # keep going\n\n\
         [[profile.default.overrides]]\nfilter = 'test(slow)'\nslow-timeout = \"5m\"\n",
        "[profile.default]\nretries = 2\n",
        "[profile.default.junit]\nstore-success-output = true\n",
        "profile.default.retries = 1\n",
        "",
    ];
    for mine in cases {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), NEXTEST_TOML, mine);
        let out = stdout_of(&mut init(dir.path(), &["--ci", "github"]));
        assert!(
            out.contains(&format!("added the JUnit stanza to {NEXTEST_TOML}")),
            "{mine:?}: {out}"
        );
        assert_eq!(
            junit_path(dir.path()).as_deref(),
            Some("junit.xml"),
            "{mine:?}"
        );
        let text = std::fs::read_to_string(dir.path().join(NEXTEST_TOML)).unwrap();
        for line in mine.lines() {
            assert!(text.contains(line), "{mine:?}: lost {line:?} in\n{text}");
        }
        if !mine.contains("[profile.default.junit]") {
            assert!(
                text.contains(&format!("{STANZA_COMMENT}[profile.default.junit]\n")),
                "{mine:?}: no comment above the new stanza in\n{text}"
            );
        }
        let mut expected = leaves(mine);
        expected.insert("profile.default.junit.path".into(), "\"junit.xml\"".into());
        assert_eq!(
            leaves(&text),
            expected,
            "{mine:?}: changed beyond the stanza"
        );
        // And a second run leaves it be.
        let again = snapshot(dir.path());
        init(dir.path(), &["--ci", "github"]).assert().success();
        assert_eq!(snapshot(dir.path()), again, "{mine:?}");
    }
}

/// Every value in the TOML `text`, by its dotted key, as TOML writes it.
/// Arrays of tables are keyed by index.
fn leaves(text: &str) -> std::collections::BTreeMap<String, String> {
    fn walk(
        prefix: &str,
        item: &toml_edit::Item,
        out: &mut std::collections::BTreeMap<String, String>,
    ) {
        let key = |k: &str| {
            if prefix.is_empty() {
                k.to_owned()
            } else {
                format!("{prefix}.{k}")
            }
        };
        if let Some(table) = item.as_table_like() {
            for (k, v) in table.iter() {
                walk(&key(k), v, out);
            }
        } else if let Some(array) = item.as_array_of_tables() {
            for (at, table) in array.iter().enumerate() {
                for (k, v) in table.iter() {
                    walk(&format!("{prefix}[{at}].{k}"), v, out);
                }
            }
        } else if let Some(value) = item.as_value() {
            out.insert(
                prefix.to_owned(),
                value.clone().decorated("", "").to_string(),
            );
        }
    }
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    let mut out = std::collections::BTreeMap::new();
    walk("", doc.as_item(), &mut out);
    out
}

#[test]
fn a_nextest_toml_that_cannot_be_read_as_toml_is_never_touched() {
    for force in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), NEXTEST_TOML, "[profile.default\nnot toml");
        let before = snapshot(dir.path());
        let mut cmd = init(dir.path(), &["--ci", "github"]);
        if force {
            cmd.arg("--force");
        }
        cmd.assert()
            .code(1)
            .stderr(predicate::str::contains(NEXTEST_TOML));
        assert_eq!(snapshot(dir.path()), before, "force={force}");
    }
}

#[test]
fn a_nextest_toml_that_is_not_toml_is_quoted_escaped() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        NEXTEST_TOML,
        "x = \"\u{1b}[31mred\u{1b}]0;title\u{7}\" oops\n",
    );
    let out = init(dir.path(), &["--ci", "github"]).assert().code(1);
    let stderr = &out.get_output().stderr;
    assert!(
        !stderr.iter().any(|&b| b == 0x1b || b == 0x07),
        "{}",
        String::from_utf8_lossy(stderr)
    );
    let stderr = String::from_utf8_lossy(stderr);
    assert!(stderr.contains("is not TOML"), "{stderr}");
    assert!(stderr.contains("\\u{1b}[31mred"), "{stderr}");
}

#[test]
fn a_junit_setting_of_the_wrong_shape_is_never_touched() {
    for mine in [
        "[profile.default]\njunit = \"junit.xml\"\n",
        "profile = 3\n",
        "[profile.default.junit]\npath = 7\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), NEXTEST_TOML, mine);
        let before = snapshot(dir.path());
        init(dir.path(), &["--ci", "github", "--force"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(NEXTEST_TOML));
        assert_eq!(snapshot(dir.path()), before, "{mine:?}");
    }
}

#[test]
fn a_conflict_anywhere_means_nothing_is_written_anywhere() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), GITHUB_WORKFLOW, "# mine\n");
    write(dir.path(), NEXTEST_TOML, "[profile.ci]\nretries = 1\n");
    let before = snapshot(dir.path());
    init(dir.path(), &["--ci", "github"]).assert().code(1);
    assert_eq!(snapshot(dir.path()), before);
}

const FORGEJO_WORKFLOW: &str = ".forgejo/workflows/greenshift.yml";

#[test]
fn forgejo_needs_the_token_limitation_accepted_and_otherwise_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &["--ci", "forgejo"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--accept-unisolated"))
        .stderr(predicate::str::contains("write access"));
    assert!(snapshot(dir.path()).is_empty());
}

#[test]
fn forgejo_writes_the_golden_workflow_and_says_it_is_not_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let out = stdout_of(&mut init(
        dir.path(),
        &["--ci", "forgejo", "--accept-unisolated"],
    ));
    assert!(out.contains(&format!("wrote {FORGEJO_WORKFLOW}")), "{out}");
    assert!(out.contains(&format!("wrote {NEXTEST_TOML}")), "{out}");
    assert!(!dir.path().join(".github").exists());
    let workflow = dir.path().join(FORGEJO_WORKFLOW);
    assert_golden(&workflow, "init-nextest-forgejo.yml");
    let text = std::fs::read_to_string(&workflow).unwrap();
    assert!(text.contains("THIS WORKFLOW IS NOT ISOLATED"), "{text}");
    assert!(
        !text.contains("permissions:"),
        "permissions would be ignored"
    );
}

#[test]
fn forgejo_is_found_from_the_repository_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".forgejo")).unwrap();
    let out = stdout_of(&mut init(dir.path(), &["--accept-unisolated"]));
    assert!(
        out.contains("found .forgejo, so writing for forgejo"),
        "{out}"
    );
    assert!(dir.path().join(FORGEJO_WORKFLOW).is_file());
    let before = snapshot(dir.path());
    let out = stdout_of(&mut init(dir.path(), &["--accept-unisolated"]));
    assert!(out.contains("nothing to change"), "{out}");
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn accepting_the_forgejo_limitation_is_refused_for_github() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path(), &["--ci", "github", "--accept-unisolated"])
        .assert()
        .code(2);
    assert!(snapshot(dir.path()).is_empty());
}

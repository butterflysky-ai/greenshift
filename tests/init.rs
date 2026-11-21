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

/// Each forge, as `--ci` names it, with where its workflow goes and the
/// golden file it matches.
const FORGES: [(&str, &str, &str); 2] = [
    ("github", GITHUB_WORKFLOW, "init-nextest-github.yml"),
    ("forgejo", FORGEJO_WORKFLOW, "init-nextest-forgejo.yml"),
];

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
    assert!(!out.contains("not isolated"), "{out}");
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
    for (forge, path, golden_name) in FORGES {
        let dir = tempfile::tempdir().unwrap();
        let fresh = fresh_mode(dir.path());
        init(dir.path(), &["--ci", forge]).assert().success();
        assert_eq!(
            mode(&dir.path().join(path)),
            fresh,
            "{forge}: a new file gets the mode any new file would"
        );
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), path, "# mine\n");
        let workflow = dir.path().join(path);
        let kept = if fresh == 0o640 { 0o600 } else { 0o640 };
        std::fs::set_permissions(&workflow, std::fs::Permissions::from_mode(kept)).unwrap();
        init(dir.path(), &["--ci", forge, "--force"])
            .assert()
            .success();
        assert_golden(&workflow, golden_name);
        assert_eq!(
            mode(&workflow),
            kept,
            "{forge}: the rewritten file keeps its mode"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_on_the_way_to_a_file_is_never_followed_or_replaced() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("credentials.toml");
    std::fs::write(&secret, "[registry]\ntoken = \"secret\"\n").unwrap();
    std::fs::create_dir(outside.path().join("workflows")).unwrap();
    // A symlinked file, and a symlinked directory on the way to one, at
    // each level, for each forge.
    let mut cases = Vec::new();
    for (forge, path, _) in FORGES {
        let workflows = path.strip_suffix("/greenshift.yml").unwrap();
        let top = workflows.strip_suffix("/workflows").unwrap();
        cases.push((forge, path, secret.clone()));
        cases.push((forge, workflows, outside.path().join("workflows")));
        cases.push((forge, top, outside.path().to_owned()));
    }
    for (forge, link, target) in cases {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join(link);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, &at).unwrap();
        let outside_before = snapshot(outside.path());
        for force in [false, true] {
            let mut cmd = init(dir.path(), &["--ci", forge]);
            if force {
                cmd.arg("--force");
            }
            cmd.assert()
                .code(1)
                .stderr(predicate::str::contains(link))
                .stderr(predicate::str::contains("symlink"));
            let meta = std::fs::symlink_metadata(&at).unwrap();
            assert!(meta.file_type().is_symlink(), "{link}: link kept");
            assert_eq!(
                snapshot(outside.path()),
                outside_before,
                "{link}: nothing is written through it"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_written_means_none_is_written() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // The workflow's directory cannot be made.
    let github = dir.path().join(".github");
    std::fs::create_dir(&github).unwrap();
    std::fs::set_permissions(&github, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(github.join("probe")).is_ok() {
        // Running as root, which can write anywhere: nothing to test.
        std::fs::set_permissions(&github, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let out = init(dir.path(), &["--ci", "github"]).assert().code(1);
    std::fs::set_permissions(&github, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(stderr.contains(GITHUB_WORKFLOW), "{stderr}");
    assert!(stderr.contains("nothing written"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(!stdout.contains("wrote"), "{stdout}");
    assert!(
        snapshot(dir.path()).is_empty(),
        "no file, staged or not, is left"
    );
}

#[test]
fn an_existing_different_workflow_is_left_alone_without_force() {
    for (forge, path, _) in FORGES {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), path, "# mine\n");
        let before = snapshot(dir.path());
        init(dir.path(), &["--ci", forge])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(path))
            .stderr(predicate::str::contains("--force"));
        assert_eq!(snapshot(dir.path()), before, "{forge}: nothing is written");
    }
}

#[test]
fn force_overwrites_the_workflow() {
    for (forge, path, golden_name) in FORGES {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), path, "# mine\n");
        let out = stdout_of(&mut init(dir.path(), &["--ci", forge, "--force"]));
        assert!(out.contains(&format!("overwrote {path}")), "{forge}: {out}");
        assert_golden(&dir.path().join(path), golden_name);
    }
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

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_nextest_toml_is_left_byte_for_byte_and_none_is_created() {
    for forge in [&["--ci", "github"][..], &["--ci", "forgejo"]] {
        let dir = tempfile::tempdir().unwrap();
        let out = stdout_of(&mut init(dir.path(), forge));
        assert!(!dir.path().join(".config").exists(), "{forge:?}");
        assert!(!out.contains("nextest.toml"), "{forge:?}: {out}");
        for mine in [
            "# mine\n[profile.default.junit]\npath = \"results/junit.xml\"\n",
            "[profile.ci]\nretries = 1\n",
            "",
            "[profile.default\nnot toml",
            "x = \"\u{1b}[31mred\" oops\n",
        ] {
            for force in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                write(dir.path(), NEXTEST_TOML, mine);
                let mut cmd = init(dir.path(), forge);
                if force {
                    cmd.arg("--force");
                }
                let out = stdout_of(&mut cmd);
                assert!(!out.contains("nextest.toml"), "{forge:?} {mine:?}: {out}");
                assert_eq!(
                    std::fs::read(dir.path().join(NEXTEST_TOML)).unwrap(),
                    mine.as_bytes(),
                    "{forge:?} {mine:?} force={force}"
                );
            }
        }
    }
}

const FORGEJO_WORKFLOW: &str = ".forgejo/workflows/greenshift.yml";

#[test]
fn forgejo_writes_the_golden_workflow_and_says_it_is_not_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let out = stdout_of(&mut init(dir.path(), &["--ci", "forgejo"]));
    assert!(out.contains(&format!("wrote {FORGEJO_WORKFLOW}")), "{out}");
    assert_eq!(
        out.lines()
            .filter(|line| line.contains("not isolated"))
            .count(),
        1,
        "{out}"
    );
    assert!(!dir.path().join(NEXTEST_TOML).exists());
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
    let out = stdout_of(&mut init(dir.path(), &[]));
    assert!(
        out.contains("found .forgejo, so writing for forgejo"),
        "{out}"
    );
    assert!(dir.path().join(FORGEJO_WORKFLOW).is_file());
    let before = snapshot(dir.path());
    let out = stdout_of(&mut init(dir.path(), &[]));
    assert!(out.contains("nothing to change"), "{out}");
    assert!(out.contains("not isolated"), "{out}");
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn force_replaces_an_existing_workflow_that_is_not_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let workflow = dir.path().join(GITHUB_WORKFLOW);
    std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    std::fs::write(&workflow, b"\xff\xfe").unwrap();
    let out = stdout_of(&mut init(dir.path(), &["--ci", "github", "--force"]));
    assert!(
        out.contains(&format!("overwrote {GITHUB_WORKFLOW}")),
        "{out}"
    );
    assert_golden(&workflow, "init-nextest-github.yml");
}

#[test]
fn an_existing_workflow_that_is_not_utf8_is_a_conflict_without_force() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), GITHUB_WORKFLOW, "");
    std::fs::write(dir.path().join(GITHUB_WORKFLOW), b"\xff\xfe").unwrap();
    let before = snapshot(dir.path());
    let out = init(dir.path(), &["--ci", "github"]).assert().code(1);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains(&format!("greenshift: {GITHUB_WORKFLOW} exists and differs")),
        "{stderr}"
    );
    assert!(stderr.contains("--force"), "{stderr}");
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn a_directory_where_the_workflow_goes_is_named_as_given_and_nothing_is_written() {
    for force in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(GITHUB_WORKFLOW)).unwrap();
        let mut cmd = init(dir.path(), &["--ci", "github"]);
        if force {
            cmd.arg("--force");
        }
        let out = cmd.assert().code(1);
        let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
        assert!(stderr.contains(GITHUB_WORKFLOW), "{stderr}");
        assert!(
            !stderr.contains("./"),
            "a relative path, as elsewhere: {stderr}"
        );
        assert!(
            dir.path().join(GITHUB_WORKFLOW).is_dir(),
            "force={force}: the directory is kept"
        );
        assert!(
            snapshot(dir.path()).is_empty(),
            "force={force}: nothing written"
        );
    }
}

#[test]
fn a_rejected_release_is_named_once() {
    let dir = tempfile::tempdir().unwrap();
    let out = greenshift(dir.path())
        .args(["init", "nextest", "--ci", "github", "--release", "v9.9"])
        .args(["--sha256", SHA256])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert_eq!(stderr.matches("v9.9").count(), 1, "{stderr}");
}

#[test]
fn a_github_file_that_is_not_a_directory_does_not_name_the_forge() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".github"), "").unwrap();
    init(dir.path(), &[])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("found neither"));
    std::fs::create_dir(dir.path().join(".forgejo")).unwrap();
    let out = stdout_of(&mut init(dir.path(), &[]));
    assert!(
        out.contains("found .forgejo, so writing for forgejo"),
        "{out}"
    );
}

/// Two wanted files, `new.txt` and `sub/other.txt`, as a backend with
/// more than one would ask for them.
#[cfg(unix)]
fn two_wants() -> Vec<greenshift::init::Want> {
    ["new.txt", "sub/other.txt"]
        .into_iter()
        .map(|path| greenshift::init::Want {
            path: path.into(),
            content: "wanted\n".into(),
        })
        .collect()
}

#[cfg(unix)]
#[test]
fn a_conflict_in_one_wanted_file_means_none_is_written() {
    use greenshift::init::{apply, decide, Action};
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "sub/other.txt", "different\n");
    let actions = decide(dir.path(), two_wants(), false).unwrap();
    assert!(matches!(actions[0], Action::Write { .. }), "{actions:?}");
    assert!(matches!(actions[1], Action::Conflict { .. }), "{actions:?}");
    let code = apply(dir.path(), &actions).unwrap();
    assert_eq!(code, std::process::ExitCode::from(1));
    assert_eq!(
        snapshot(dir.path()),
        vec![(PathBuf::from("sub/other.txt"), b"different\n".to_vec())],
        "the file without a conflict is not written either"
    );
}

#[cfg(unix)]
#[test]
fn one_wanted_file_that_cannot_be_staged_means_none_is_written() {
    use greenshift::init::{apply, decide};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // The second file cannot be staged in its directory; the first can.
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(sub.join("probe")).is_ok() {
        // Running as root, which can write anywhere: nothing to test.
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let actions = decide(dir.path(), two_wants(), false).unwrap();
    let result = apply(dir.path(), &actions);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = result.expect_err("staging sub/other.txt fails");
    assert!(format!("{err:#}").contains("nothing written"), "{err:#}");
    assert!(
        snapshot(dir.path()).is_empty(),
        "new.txt, staged first, is neither renamed into place nor left staged"
    );
}

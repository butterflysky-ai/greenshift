//! Runs `scripts/check-release-tag.sh`, which the release workflow uses
//! to check a tag against the version in Cargo.toml. Needs bash and awk.

use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

fn script() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/check-release-tag.sh")
}

/// The script on `tag` and a manifest holding `toml`.
fn check(tag: &str, toml: &str) -> assert_cmd::assert::Assert {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(&manifest, toml).unwrap();
    Command::new("bash")
        .arg(script())
        .arg(tag)
        .arg(&manifest)
        .assert()
}

#[test]
fn the_projects_own_version_matches_its_tag() {
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    Command::new("bash")
        .arg(script())
        .arg(&tag)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .assert()
        .success()
        .stdout(contains(format!("tag {tag} matches")));
    Command::new("bash")
        .arg(script())
        .arg("v0.0.0-not-this")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .assert()
        .code(1)
        .stdout(contains("does not match"));
}

#[test]
fn the_package_version_is_read() {
    for toml in [
        "[package]\nname = \"a\"\nversion = \"1.2.3\"\n",
        "[package] # the crate\nversion = \"1.2.3\" # bumped\n",
        "[package]\nversion='1.2.3'\n",
        "[ package ]\n  version=\"1.2.3\"\n",
        "[package]\r\nversion = \"1.2.3\"\r\n",
        "[dependencies]\nversion = \"9\"\n[package]\nversion = \"1.2.3\"\n",
    ] {
        check("v1.2.3", toml).success().stdout(contains("matches"));
        check("v1.2.4", toml)
            .code(1)
            .stdout(contains("does not match Cargo.toml version 1.2.3"));
        check("1.2.3", toml).code(1);
    }
}

#[test]
fn a_pre_release_version_is_read_whole() {
    let toml = "[package]\nversion = \"0.2.0-rc.1\"\n";
    check("v0.2.0-rc.1", toml).success();
    check("v0.2.0", toml).code(1);
}

#[test]
fn a_version_outside_package_is_not_taken() {
    for toml in [
        "[package]\nname = \"a\"\n[package.metadata]\nversion = \"1.2.3\"\n",
        "[package]\nname = \"a\"\n[dependencies]\nversion = \"1.2.3\"\n",
        "[workspace.package]\nversion = \"1.2.3\"\n",
        "[package]\n# version = \"1.2.3\"\n",
        "[package]\nversion.workspace = true\n",
        "[package]\nversion = \"1.2.3\n",
    ] {
        check("v1.2.3", toml)
            .code(1)
            .stdout(contains("no [package] version"));
    }
}

#[test]
fn a_missing_tag_is_a_usage_error() {
    for args in [&[][..], &["v1", "Cargo.toml", "extra"]] {
        Command::new("bash")
            .arg(script())
            .args(args)
            .assert()
            .code(2)
            .stdout("")
            .stderr(contains("usage:"));
    }
}

//! What the workflows `greenshift init nextest` writes do: the trust
//! boundary between the test job and the publisher, as the durable-history
//! reference workflow draws it, and the install step refusing a binary
//! that does not match the pinned checksum. Needs bash, sha256sum and
//! mikefarah's yq v4, as the other workflow tests do.

use std::process::Command;

use serde_json::Value;

const RELEASE: &str = "v0.2.0";

/// The workflow `init nextest --ci <forge>` writes, as JSON, with `sha256`
/// pinned, and `extra` arguments.
fn generated(forge: &str, sha256: &str, extra: &[&str]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    assert_cmd::Command::cargo_bin("greenshift")
        .unwrap()
        .current_dir(dir.path())
        .args(["init", "nextest", "--ci", forge, "--release", RELEASE])
        .args(["--sha256", sha256])
        .args(extra)
        .assert()
        .success();
    let path = dir
        .path()
        .join(format!(".{forge}/workflows/greenshift.yml"));
    let output = Command::new("yq")
        .args(["-o=json", "."])
        .arg(&path)
        .output()
        .expect("mikefarah yq installed as for cache_guard tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn steps<'w>(workflow: &'w Value, job: &str) -> &'w Vec<Value> {
    workflow["jobs"][job]["steps"].as_array().unwrap()
}

fn step<'w>(workflow: &'w Value, job: &str, name: &str) -> &'w Value {
    steps(workflow, job)
        .iter()
        .find(|step| step["name"] == name)
        .unwrap_or_else(|| panic!("no step {name:?} in {job}"))
}

/// The jobs and steps that make the reference's boundary, whatever the
/// forge: the publisher needs the test job, runs only for pushes to the
/// default branch, never checks out or builds test code, and only
/// greenshift steps see the history token.
fn assert_publisher_separation(workflow: &Value) {
    let publisher = &workflow["jobs"]["publish"];
    assert_eq!(publisher["needs"], "test");
    let condition = publisher["if"].as_str().unwrap();
    assert!(condition.contains("github.event_name == 'push'"));
    assert!(condition.contains("github.event.repository.default_branch"));
    for step in steps(workflow, "publish") {
        assert!(!step["uses"].as_str().unwrap_or("").contains("checkout"));
        assert!(!step["run"].as_str().unwrap_or("").contains("cargo"));
    }
    assert!(workflow["env"]["GREENSHIFT_HISTORY_TOKEN"].is_null());
    for job in ["test", "publish"] {
        assert!(workflow["jobs"][job]["env"]["GREENSHIFT_HISTORY_TOKEN"].is_null());
        for step in steps(workflow, job) {
            if !step["env"]["GREENSHIFT_HISTORY_TOKEN"].is_null() {
                assert!(step["run"].as_str().unwrap().contains("greenshift"));
                assert_eq!(
                    step["env"]["GREENSHIFT_HISTORY_TOKEN"],
                    "${{ github.token }}"
                );
            }
        }
    }
    let checkout = steps(workflow, "test")
        .iter()
        .find(|step| step["uses"].as_str().unwrap_or("").contains("checkout@"))
        .unwrap();
    assert_eq!(checkout["with"]["persist-credentials"], false);
    let save = steps(workflow, "publish")
        .iter()
        .find(|step| step["id"] == "save")
        .unwrap();
    assert!(save["if"]
        .as_str()
        .unwrap()
        .contains("steps.artifact.outcome == 'success'"));
    assert!(save["run"]
        .as_str()
        .unwrap()
        .contains("--default-branch \"$DEFAULT_BRANCH\""));
    for (job, id) in [
        ("test", "cache"),
        ("publish", "artifact"),
        ("publish", "cache"),
    ] {
        let step = steps(workflow, job)
            .iter()
            .find(|step| step["id"] == id)
            .unwrap();
        assert_eq!(step["continue-on-error"], true, "{job}.{id}");
    }
    let run = step(workflow, "test", "Run tests and export data")["run"]
        .as_str()
        .unwrap();
    assert!(run.contains("run nextest --history-remote"));
    assert!(run.contains("--history-artifact .greenshift/publish.sqlite"));
    // Every action is pinned to a commit.
    for job in ["test", "publish"] {
        for step in steps(workflow, job) {
            if let Some(uses) = step["uses"].as_str() {
                let (_, rev) = uses.rsplit_once('@').unwrap();
                assert!(
                    rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{uses} is not pinned to a commit"
                );
            }
        }
    }
}

#[test]
fn github_keeps_the_reference_trust_boundary() {
    let workflow = generated("github", &"a".repeat(64), &[]);
    assert_publisher_separation(&workflow);
    assert_eq!(workflow["permissions"]["contents"], "read");
    assert_eq!(workflow["jobs"]["test"]["permissions"]["contents"], "read");
    assert_eq!(
        workflow["jobs"]["publish"]["permissions"]["contents"],
        "write"
    );
}

/// Run the install step's script of `job`, with `curl` replaced by one
/// that writes `binary` where the real one would write the download, and
/// return whether it succeeded and what it installed.
fn install(workflow: &Value, job: &str, binary: &[u8]) -> (bool, Option<Vec<u8>>) {
    let script = step(workflow, job, "Install pinned greenshift release")["run"]
        .as_str()
        .unwrap()
        .to_owned();
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let payload = dir.path().join("payload");
    std::fs::write(&payload, binary).unwrap();
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/usr/bin/env bash\nwhile [[ $# -gt 0 ]]; do\n  \
             if [[ $1 == --output ]]; then cp {payload:?} \"$2\"; exit 0; fi\n  \
             shift\ndone\nexit 22\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let temp = dir.path().join("runner-temp");
    std::fs::create_dir(&temp).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let env = workflow["env"].as_object().unwrap();
    let status = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &script])
        .env("PATH", path)
        .env("RUNNER_TEMP", &temp)
        .env(
            "GREENSHIFT_RELEASE",
            env["GREENSHIFT_RELEASE"].as_str().unwrap(),
        )
        .env(
            "GREENSHIFT_SHA256",
            env["GREENSHIFT_SHA256"].as_str().unwrap(),
        )
        .status()
        .unwrap();
    let installed = temp.join("greenshift");
    let leftovers: Vec<_> = std::fs::read_dir(&temp)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "greenshift")
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    (
        status.success(),
        installed
            .exists()
            .then(|| std::fs::read(installed).unwrap()),
    )
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
#[test]
fn the_install_step_fails_closed_on_a_checksum_mismatch() {
    let binary = b"#!/bin/sh\necho pretend greenshift\n";
    let pinned = sha256_hex(binary);
    for forge in forges() {
        let workflow = generated(forge, &pinned, extra_for(forge));
        for job in ["test", "publish"] {
            let (ok, installed) = install(&workflow, job, binary);
            assert!(ok, "{forge} {job}: the matching binary is installed");
            assert_eq!(installed.as_deref(), Some(&binary[..]));
            let (ok, installed) = install(&workflow, job, b"tampered");
            assert!(!ok, "{forge} {job}: a mismatch fails the step");
            assert_eq!(installed, None, "{forge} {job}: nothing is installed");
        }
    }
}

/// The forges `init nextest` writes for.
fn forges() -> &'static [&'static str] {
    &["github"]
}

/// The arguments `init nextest --ci forge` needs beyond the pin.
fn extra_for(_forge: &str) -> &'static [&'static str] {
    &[]
}

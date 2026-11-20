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
    let urls = dir.path().join("urls");
    fake_curl(&bin, &payload, &urls);
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
    let release = env["GREENSHIFT_RELEASE"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(&urls).unwrap(),
        format!(
            "https://github.com/paceheart/greenshift/releases/download/{release}/\
             greenshift-{release}-x86_64-unknown-linux-musl\n"
        ),
        "the one download is the pinned release's Linux binary"
    );
    (
        status.success(),
        installed
            .exists()
            .then(|| std::fs::read(installed).unwrap()),
    )
}

/// An executable `curl` in `bin` that writes `payload` where the real one
/// would write the download, and appends each URL it is given to `urls`.
/// Like the real one with --fail, it fails without an --output.
fn fake_curl(bin: &std::path::Path, payload: &std::path::Path, urls: &std::path::Path) {
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/usr/bin/env bash\nout=\nwhile [[ $# -gt 0 ]]; do\n  \
             case $1 in\n    --output) out=$2; shift ;;\n    \
             https://*) echo \"$1\" >> {urls:?} ;;\n  esac\n  \
             shift\ndone\n[[ -n $out ]] || exit 22\ncp {payload:?} \"$out\"\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
    &["github", "forgejo"]
}

/// The arguments `init nextest --ci forge` needs beyond the pin.
fn extra_for(forge: &str) -> &'static [&'static str] {
    match forge {
        "forgejo" => &["--accept-unisolated"],
        _ => &[],
    }
}

#[test]
fn forgejo_keeps_the_publisher_apart_but_claims_no_permissions() {
    let workflow = generated("forgejo", &"a".repeat(64), &["--accept-unisolated"]);
    assert_publisher_separation(&workflow);
    // Forgejo ignores `permissions`; writing them would claim an
    // isolation the test job does not have.
    assert!(workflow["permissions"].is_null());
    for job in ["test", "publish"] {
        assert!(workflow["jobs"][job]["permissions"].is_null(), "{job}");
        for step in steps(&workflow, job) {
            if let Some(uses) = step["uses"].as_str() {
                // Not resolved against the instance's DEFAULT_ACTIONS_URL.
                assert!(uses.starts_with("https://"), "{uses}");
                if uses.contains("artifact@") {
                    // Stock v4 artifact actions refuse to run on Forgejo.
                    assert!(uses.contains("/forgejo/"), "{uses}");
                }
            }
        }
    }
    let rust = step(&workflow, "test", "Install Rust when the runner has none");
    assert!(rust["run"].as_str().unwrap().contains("sha256sum --check"));
}

#[test]
fn every_runner_label_reaches_the_workflow_as_that_string() {
    // Each would be something else, or no YAML, if written bare.
    for label in [
        "-",
        ":",
        "a:",
        "null",
        "true",
        "0x1F",
        ".inf",
        "ubuntu-24.04",
    ] {
        for forge in forges() {
            let mut extra = extra_for(forge).to_vec();
            extra.extend(["--runs-on", label]);
            let workflow = generated(forge, &"a".repeat(64), &extra);
            for job in ["test", "publish"] {
                assert_eq!(
                    workflow["jobs"][job]["runs-on"],
                    Value::String(label.into()),
                    "{forge} {job} {label:?}"
                );
            }
        }
    }
}

/// `text` with whitespace runs made single spaces.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_forgejo_header_quotes_the_capability_gate_as_the_design_says_it() {
    let root = env!("CARGO_MANIFEST_DIR");
    let doc = std::fs::read_to_string(format!("{root}/docs/design/durable-history.md")).unwrap();
    let doc = words(&doc.replace('`', "").replace("**", ""));
    let dir = tempfile::tempdir().unwrap();
    assert_cmd::Command::cargo_bin("greenshift")
        .unwrap()
        .current_dir(dir.path())
        .args(["init", "nextest", "--ci", "forgejo", "--accept-unisolated"])
        .args(["--release", RELEASE, "--sha256", &"a".repeat(64)])
        .assert()
        .success();
    let text =
        std::fs::read_to_string(dir.path().join(".forgejo/workflows/greenshift.yml")).unwrap();
    // The quoted paragraphs: the comment lines indented under "says:".
    let mut quoted = vec![String::new()];
    let mut inside = false;
    for line in text.lines().take_while(|line| line.starts_with('#')) {
        let body = line.trim_start_matches('#');
        if body.trim_end().ends_with("says:") {
            inside = true;
        } else if inside && body.starts_with("   ") {
            quoted.last_mut().unwrap().push_str(&format!(" {body}"));
        } else if inside && body.trim().is_empty() {
            quoted.push(String::new());
        } else if inside {
            break;
        }
    }
    let quoted: Vec<String> = quoted
        .iter()
        .map(|q| words(q))
        .filter(|q| !q.is_empty())
        .collect();
    assert!(quoted.len() >= 2, "{quoted:?}");
    for paragraph in &quoted {
        assert!(
            doc.contains(paragraph.as_str()),
            "not in the design: {paragraph:?}"
        );
    }
    // Whole sentences, none dropped from the first paragraph.
    assert!(
        quoted[0].ends_with("read-only-test-job guarantee."),
        "{:?}",
        quoted[0]
    );
}

/// Run the Forgejo test job's Rust step with a PATH that has only the
/// tools it needs, `cargo` among them when `with_cargo`, and a `curl`
/// that downloads `payload`; return whether it succeeded, the URLs it
/// fetched, and whether the download was run.
#[cfg(unix)]
fn install_rust(with_cargo: bool, payload: &[u8]) -> (bool, String, bool) {
    let workflow = generated("forgejo", &"a".repeat(64), &["--accept-unisolated"]);
    let script = step(&workflow, "test", "Install Rust when the runner has none")["run"]
        .as_str()
        .unwrap()
        .to_owned();
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for tool in ["bash", "sha256sum", "rm", "chmod", "env", "cp", "touch"] {
        let found = Command::new("bash")
            .args(["-c", &format!("command -v {tool}")])
            .output()
            .unwrap();
        let found = String::from_utf8(found.stdout).unwrap();
        std::os::unix::fs::symlink(found.trim(), bin.join(tool)).unwrap();
    }
    let ran = dir.path().join("ran");
    if with_cargo {
        let cargo = bin.join("cargo");
        std::fs::write(&cargo, "#!/usr/bin/env bash\necho cargo 1.0.0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let download = dir.path().join("payload");
    std::fs::write(&download, payload).unwrap();
    let urls = dir.path().join("urls");
    fake_curl(&bin, &download, &urls);
    let temp = dir.path().join("runner-temp");
    std::fs::create_dir(&temp).unwrap();
    let status = Command::new(bin.join("bash"))
        .args(["--noprofile", "--norc", "-c", &script])
        .env_clear()
        .env("PATH", &bin)
        .env("RUNNER_TEMP", &temp)
        .env("RAN", &ran)
        .env("GITHUB_PATH", dir.path().join("github-path"))
        .status()
        .unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(&temp).unwrap().collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    (
        status.success(),
        std::fs::read_to_string(&urls).unwrap_or_default(),
        ran.exists(),
    )
}

#[cfg(unix)]
#[test]
fn the_rust_step_uses_the_runners_cargo_and_otherwise_installs_only_a_matching_rustup() {
    let rustup = b"#!/usr/bin/env bash\ntouch \"$RAN\"\n";
    let (ok, urls, ran) = install_rust(true, rustup);
    assert!(
        ok && urls.is_empty() && !ran,
        "cargo present: nothing fetched"
    );
    let (ok, urls, ran) = install_rust(false, rustup);
    assert!(
        !ok,
        "a rustup-init that does not match the pin fails the step"
    );
    assert!(!ran, "and is never run");
    assert_eq!(
        urls,
        "https://static.rust-lang.org/rustup/archive/1.29.3/x86_64-unknown-linux-gnu/rustup-init\n"
    );
}

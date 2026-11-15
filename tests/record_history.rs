//! Runs `scripts/record-history.sh`, as CI's record step does, with every
//! pass's JUnit present, some of it, and none. Needs bash.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::str::contains;
use rusqlite::Connection;

const COMMIT: &str = "0123abcd";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

/// A temp dir with the two passes' JUnit paths in it, neither written.
struct Job {
    dir: tempfile::TempDir,
    rich: PathBuf,
    poor: PathBuf,
}

impl Job {
    fn new() -> Job {
        let dir = tempfile::tempdir().unwrap();
        let rich = dir.path().join("ci-rich/junit.xml");
        let poor = dir.path().join("ci-poor/junit.xml");
        Job { dir, rich, poor }
    }

    /// Write `pass`'s JUnit from the fixture `name`.
    fn wrote(&self, pass: &Path, name: &str) {
        std::fs::create_dir_all(pass.parent().unwrap()).unwrap();
        std::fs::copy(fixture(name), pass).unwrap();
    }

    fn db(&self) -> PathBuf {
        self.dir.path().join("history.sqlite")
    }

    /// The script, as CI calls it, on both passes' paths.
    fn record(&self) -> assert_cmd::assert::Assert {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/record-history.sh");
        Command::new("bash")
            .arg(script)
            .arg(env!("CARGO_BIN_EXE_greenshift"))
            .arg(self.db())
            .args([COMMIT, "nextest"])
            .arg(&self.rich)
            .arg(&self.poor)
            .assert()
    }

    /// Every run recorded: (partial, commit, source, result count).
    fn runs(&self) -> Vec<(bool, String, String, i64)> {
        let conn = Connection::open(self.db()).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT u.partial, u.commit_sha, u.source,
                        (SELECT count(*) FROM results r WHERE r.run_id = u.id)
                 FROM runs u ORDER BY u.id",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

fn run(partial: bool, results: i64) -> (bool, String, String, i64) {
    (partial, COMMIT.to_owned(), "nextest".to_owned(), results)
}

#[test]
fn both_passes_are_recorded_as_one_whole_run() {
    let job = Job::new();
    job.wrote(&job.rich, "vaelii-run-vars.xml");
    job.wrote(&job.poor, "nextest.xml");
    job.record()
        .success()
        .stdout(contains("recorded run 1 with 13 tests from 2 files"));
    assert_eq!(job.runs(), [run(false, 13)]);
}

#[test]
fn one_pass_alone_is_recorded_as_a_partial_run() {
    for (rich, poor, results) in [
        (None, Some("nextest.xml"), 6),
        (Some("vaelii-run-vars.xml"), None, 7),
    ] {
        let job = Job::new();
        if let Some(name) = rich {
            job.wrote(&job.rich, name);
        }
        if let Some(name) = poor {
            job.wrote(&job.poor, name);
        }
        let missing = if rich.is_none() { &job.rich } else { &job.poor };
        job.record()
            .success()
            .stdout(contains(format!("no JUnit at {}", missing.display())))
            .stdout(contains("recording 1 of 2 passes, as a partial run"))
            .stdout(contains("recorded partial run 1 with"));
        assert_eq!(job.runs(), [run(true, results)]);
    }
}

#[test]
fn no_pass_records_nothing_and_leaves_the_store_alone() {
    let job = Job::new();
    job.record()
        .success()
        .stdout(contains("no JUnit to record"));
    assert!(!job.db().exists());
}

#[test]
fn a_failed_ingest_fails_the_script() {
    let job = Job::new();
    job.wrote(&job.rich, "malformed.xml");
    job.wrote(&job.poor, "nextest.xml");
    job.record().code(1);
}

#[test]
fn too_few_arguments_is_a_usage_error() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/record-history.sh");
    Command::new("bash")
        .arg(script)
        .args(["greenshift", "db", COMMIT, "nextest"])
        .assert()
        .code(2)
        .stderr(contains("usage:"));
}

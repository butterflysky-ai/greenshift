use std::path::Path;
use std::process::Command;

use greenshift::history::Remote;
use greenshift::store::{NewRun, Store};
use rusqlite::Connection;

fn repository(path: &Path) {
    assert!(Command::new("git")
        .args(["init", "--bare"])
        .arg(path)
        .output()
        .unwrap()
        .status
        .success());
}

fn record(path: &Path, name: &str) {
    let mut store = Store::open(path).unwrap();
    store
        .record_run(
            &NewRun {
                commit: None,
                source: name,
                files: &[],
                partial: false,
                branch: Some("main"),
                tag: None,
            },
            std::iter::empty(),
        )
        .unwrap();
}

fn count(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn publish_then_wipe_cache_restores_every_run_without_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("remote.git");
    repository(&repo);
    let data = dir.path().join("input.sqlite");
    record(&data, "first");
    record(&data, "second");
    let cache = dir.path().join("cache");
    let remote = Remote::new(repo.to_str().unwrap()).unwrap();
    let oid = remote.publish(&data, Some(&cache)).unwrap();
    assert_eq!(remote.head().unwrap(), Some(oid));
    std::fs::remove_dir_all(cache).unwrap();
    let restored = dir.path().join("restored.sqlite");
    assert!(remote.restore(&restored, None).unwrap());
    assert_eq!(count(&restored), 2);
    // A replacement commit has no parents: one live full snapshot, not a log.
    let parents = Command::new("git")
        .arg("--git-dir")
        .arg(&repo)
        .args(["rev-list", "--parents", "-1", "refs/greenshift/history"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(parents.stdout)
            .unwrap()
            .split_whitespace()
            .count(),
        1
    );
}

#[test]
fn repeated_and_stale_full_snapshots_merge_without_loss_or_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("remote.git");
    repository(&repo);
    let first = dir.path().join("first.sqlite");
    record(&first, "baseline");
    let remote = Remote::new(repo.to_str().unwrap()).unwrap();
    remote.publish(&first, None).unwrap();
    let a = dir.path().join("a.sqlite");
    let b = dir.path().join("b.sqlite");
    remote.restore(&a, None).unwrap();
    remote.restore(&b, None).unwrap();
    record(&a, "writer-a");
    record(&b, "writer-b");
    remote.publish(&a, None).unwrap();
    remote.publish(&b, None).unwrap();
    remote.publish(&a, None).unwrap();
    let restored = dir.path().join("read.sqlite");
    remote.restore(&restored, None).unwrap();
    assert_eq!(count(&restored), 3);
}

#[test]
fn corrupt_or_substituted_cache_falls_back_to_ref_and_tampered_artifact_cannot_publish() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("remote.git");
    repository(&repo);
    let db = dir.path().join("db.sqlite");
    record(&db, "real");
    let remote = Remote::new(repo.to_str().unwrap()).unwrap();
    let cache = dir.path().join("cache");
    let oid = remote.publish(&db, Some(&cache)).unwrap();
    std::fs::write(cache.join(&oid).join("history.sqlite.gz"), b"not a gzip").unwrap();
    let restored = dir.path().join("read.sqlite");
    remote.restore(&restored, Some(&cache)).unwrap();
    assert_eq!(count(&restored), 1);
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TRIGGER evil AFTER INSERT ON runs BEGIN DELETE FROM results; END;")
        .unwrap();
    assert!(remote.publish(&db, None).is_err());
    assert_eq!(remote.head().unwrap(), Some(oid));
}

#[test]
fn cache_key_and_storage_failure_do_not_fail_cli_and_pr_cannot_publish() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("remote.git");
    repository(&repo);
    assert_cmd::Command::cargo_bin("greenshift")
        .unwrap()
        .args(["history", "cache-key", "--remote"])
        .arg(&repo)
        .assert()
        .success()
        .stdout("greenshift-history-empty\n");
    assert_cmd::Command::cargo_bin("greenshift")
        .unwrap()
        .args(["history", "restore", "--remote", "/nonexistent/history.git"])
        .assert()
        .success()
        .stderr(predicates::str::contains("⚠"));
    let db = dir.path().join("db.sqlite");
    record(&db, "fork");
    assert_cmd::Command::cargo_bin("greenshift")
        .unwrap()
        .env("GITHUB_EVENT_NAME", "pull_request")
        .args(["history", "publish", "--remote"])
        .arg(&repo)
        .arg("--db")
        .arg(&db)
        .assert()
        .success()
        .stderr(predicates::str::contains("refused"));
    assert!(Remote::new(repo.to_str().unwrap())
        .unwrap()
        .head()
        .unwrap()
        .is_none());
}

#[test]
fn remote_rejects_helpers_and_embedded_credentials() {
    assert!(Remote::new("evil::some-command").is_err());
    assert!(Remote::new("https://secret@example.invalid/repo").is_err());
}

#[test]
fn simultaneous_publishers_keep_both_new_runs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("remote.git");
    repository(&repo);
    let a = dir.path().join("a.sqlite");
    let b = dir.path().join("b.sqlite");
    record(&a, "a");
    record(&b, "b");
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        for data in [&a, &b] {
            let repo = &repo;
            let barrier = &barrier;
            scope.spawn(move || {
                let remote = Remote::new(repo.to_str().unwrap()).unwrap();
                barrier.wait();
                remote.publish(data, None).unwrap();
            });
        }
    });
    let result = dir.path().join("read.sqlite");
    Remote::new(repo.to_str().unwrap())
        .unwrap()
        .restore(&result, None)
        .unwrap();
    assert_eq!(count(&result), 2);
}

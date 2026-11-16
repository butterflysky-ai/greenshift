use super::*;
use crate::cli::{parse, Backend as BackendCommand, Command};
use crate::plan::HistoryState;

fn test(suite: &str, name: &str) -> InventoryTest {
    InventoryTest {
        suite: Tainted::raw(suite),
        name: Tainted::raw(name),
    }
}

#[test]
fn both_groups_run_rich_first() {
    let (a, b) = (test("s", "a"), test("s", "b"));
    let (rich, poor) = ([&a], [&b]);
    let groups = planned_groups(&rich, &poor);
    let classes: Vec<Class> = groups.iter().map(|(class, _)| *class).collect();
    assert_eq!(classes, [Class::Rich, Class::Poor]);
    assert_eq!(groups[0].1, [&a]);
    assert_eq!(groups[1].1, [&b]);
}

#[test]
fn an_empty_group_is_not_run() {
    let a = test("s", "a");
    let classes = |rich: &[&InventoryTest], poor: &[&InventoryTest]| -> Vec<Class> {
        planned_groups(rich, poor).iter().map(|g| g.0).collect()
    };
    assert_eq!(classes(&[&a], &[]), [Class::Rich]);
    assert_eq!(classes(&[], &[&a]), [Class::Poor]);
    // With no test at all, rich runs empty, so the runner decides.
    let none = planned_groups(&[], &[]);
    assert_eq!(none.len(), 1);
    assert_eq!(none[0].0, Class::Rich);
    assert!(none[0].1.is_empty());
}

#[test]
fn the_exit_status_is_the_first_failure() {
    assert_eq!(combined([]), 0);
    assert_eq!(combined([0, 0]), 0);
    assert_eq!(combined([100, 0]), 100);
    assert_eq!(combined([0, 100]), 100);
    assert_eq!(combined([0, 101, 100]), 101);
    assert_eq!(combined([4, 100]), 4);
}

#[cfg(unix)]
#[test]
fn a_process_exit_status_becomes_greenshifts() {
    use std::os::unix::process::ExitStatusExt;
    // Wait statuses: the code in the second byte, a signal in the first.
    assert_eq!(exit_code(ExitStatus::from_raw(0)), 0);
    assert_eq!(exit_code(ExitStatus::from_raw(100 << 8)), 100);
    assert_eq!(exit_code(ExitStatus::from_raw(9)), 128 + 9);
}

fn options(args: &[&str]) -> RunOptions {
    let argv = ["greenshift", "run", "nextest"].iter().chain(args);
    let Command::Run(run) = parse(argv).unwrap().command else {
        panic!("not run");
    };
    let BackendCommand::Nextest(args) = run.backend;
    args.options
}

#[test]
fn the_run_is_recorded_as_from_its_source() {
    let opts = options(&[]);
    assert_eq!(opts.select.source, ["nextest"]);
    assert_eq!(record_source(&opts, "nextest"), "nextest");
    let opts = options(&["--source", "ci-linux"]);
    assert_eq!(opts.select.source, ["ci-linux"]);
    assert_eq!(record_source(&opts, "nextest"), "ci-linux");
}

#[test]
fn a_store_that_cannot_be_read_plans_every_test_rich() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("history.sqlite");
    std::fs::write(&db, "not a SQLite file, but long enough to be read as one").unwrap();
    let opts = options(&["--db", db.to_str().unwrap()]);
    let tests = [test("s", "a"), test("s", "b")];
    let (history, groups) = cli::plan_groups(&opts.select, &tests).unwrap();
    assert_eq!(history, HistoryState::Error);
    assert_eq!(groups.rich, tests.iter().collect::<Vec<_>>());
    assert!(groups.poor.is_empty());
}

#[test]
fn no_store_plans_every_test_rich() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("new/history.sqlite");
    let opts = options(&["--db", db.to_str().unwrap()]);
    let tests = [test("s", "a")];
    let (history, groups) = cli::plan_groups(&opts.select, &tests).unwrap();
    assert_eq!(history, HistoryState::Ok);
    assert_eq!(groups.rich, [&tests[0]]);
    assert!(groups.poor.is_empty());
}

#[test]
fn run_takes_options_and_arguments_after_a_separator() {
    let argv = [
        "greenshift",
        "run",
        "nextest",
        "--db",
        "h.sqlite",
        "--commit",
        "ABCDEF1",
        "--min-runs",
        "3",
        "--max-filterset-bytes",
        "500",
        "--",
        "--no-fail-fast",
        "-E",
        "test(x)",
    ];
    let Command::Run(run) = parse(argv).unwrap().command else {
        panic!("not run");
    };
    let BackendCommand::Nextest(args) = run.backend;
    assert_eq!(args.options.select.db, PathBuf::from("h.sqlite"));
    assert_eq!(
        args.options.commit.as_ref().map(|c| c.as_str()),
        Some("abcdef1")
    );
    assert_eq!(args.options.select.min_runs, 3);
    assert_eq!(args.max_filterset_bytes, 500);
    assert_eq!(args.args, ["--no-fail-fast", "-E", "test(x)"]);
    // Arguments for nextest come after `--`, and a backend is named.
    assert!(parse(["greenshift", "run", "nextest", "--no-fail-fast"]).is_err());
    assert!(parse(["greenshift", "run"]).is_err());
    assert!(parse(["greenshift", "run", "lein"]).is_err());
    assert!(parse(["greenshift", "run", "nextest", "--branch", ""]).is_err());
    assert!(parse(["greenshift", "run", "nextest", "--tag", ""]).is_err());
    let both = ["--branch", "main", "--tag", "v1"];
    assert!(parse(["greenshift", "run", "nextest"].into_iter().chain(both)).is_err());
}

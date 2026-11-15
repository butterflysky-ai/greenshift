//! Runs `scripts/check-cache-guard.sh` on fixture workflows. Needs bash
//! and mikefarah's yq v4.

use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

/// A workflow that passes: restore anywhere, save only on push to main.
const GOOD: &str = r#"
on:
  push:
    branches: [main]
  pull_request:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - id: restore
        uses: actions/cache/restore@v4
        with:
          path: db
          key: k
      - if: always() && github.event_name == 'push'
        uses: actions/cache/save@v4
        with:
          path: db
          key: ${{ steps.restore.outputs.cache-primary-key }}
"#;

/// `scripts/check-cache-guard.sh` on `workflow`, written to a temp file.
fn guard(workflow: &str) -> assert_cmd::assert::Assert {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ci.yml");
    std::fs::write(&file, workflow).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/check-cache-guard.sh");
    Command::new("bash").arg(script).arg(&file).assert()
}

/// [`GOOD`] with `from` replaced by `to`, which must be in it once.
fn good_with(from: &str, to: &str) -> String {
    assert_eq!(GOOD.matches(from).count(), 1, "{from:?}");
    GOOD.replace(from, to)
}

#[test]
fn a_save_only_on_push_to_main_passes() {
    guard(GOOD)
        .success()
        .stdout(contains("1 cache save step(s), each only on push to main"))
        .stderr("");
}

#[test]
fn the_combined_cache_action_fails() {
    let workflow = good_with("actions/cache/restore@v4", "actions/cache@v4");
    guard(&workflow).code(1).stdout("").stderr(contains(
        "1 step(s) use actions/cache, which saves on pull requests too",
    ));
}

#[test]
fn a_save_on_any_other_condition_fails() {
    let workflow = good_with("always() && github.event_name == 'push'", "always()");
    guard(&workflow)
        .code(1)
        .stdout("")
        .stderr(contains("an actions/cache/save step runs if 'always()'"));
    let workflow = good_with(
        "      - if: always() && github.event_name == 'push'\n",
        "      -\n",
    );
    guard(&workflow)
        .code(1)
        .stdout("")
        .stderr(contains("an actions/cache/save step runs if '(always)'"));
}

#[test]
fn no_save_step_fails() {
    let workflow = good_with("actions/cache/save@v4", "actions/upload-artifact@v4");
    guard(&workflow)
        .code(1)
        .stdout("")
        .stderr(contains("no actions/cache/save step to check"));
}

#[test]
fn push_events_on_other_branches_fail() {
    let workflow = good_with("branches: [main]", "branches: [main, release]");
    guard(&workflow).code(1).stdout("").stderr(contains(
        r#"push events run on branches ["main","release"], not only main"#,
    ));
}

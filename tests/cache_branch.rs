//! Runs `scripts/cache-branch.sh`, which cleans up a branch name for an
//! actions/cache key, and checks CI's key uses it. Needs bash, sha256sum
//! and mikefarah's yq v4.

use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

/// The longest name the script prints.
const MAX: usize = 200;

fn script() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/cache-branch.sh")
}

/// The script's output for `branch`, without the newline.
fn clean(branch: &str) -> String {
    let out = Command::new("bash")
        .arg(script())
        .arg(branch)
        .assert()
        .success()
        .stderr("");
    let out = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    out.strip_suffix('\n').unwrap().to_owned()
}

/// The checks actions/cache makes of a key (checkKey in actions/toolkit's
/// packages/cache/src/cache.ts), on the key CI builds, with a run id and
/// attempt as long as they get.
fn assert_valid_key(name: &str) {
    let key = format!("greenshift-db-{name}-9223372036854775807-999999");
    assert!(key.chars().count() <= 512, "{key}");
    assert!(!key.contains(','), "{key}");
    assert!(!key.contains('\n'), "{key}");
}

/// `name` is `prefix`, "-" and 16 hex digits.
fn assert_hashed(name: &str, prefix: &str) {
    let (head, hash) = name.rsplit_once('-').unwrap();
    assert_eq!(head, prefix);
    assert_eq!(hash.len(), 16, "{name}");
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
}

#[test]
fn ordinary_branch_names_are_kept_as_they_are() {
    for branch in ["main", "lain/dogfood-ci", "feat/x+y@z.1", "v1.2_rc-3"] {
        assert_eq!(clean(branch), branch);
    }
    let longest = "b".repeat(MAX);
    assert_eq!(clean(&longest), longest);
}

#[test]
fn a_comma_is_replaced_and_the_name_hashed() {
    let name = clean("fix,typo");
    assert_hashed(&name, "fix_typo");
    assert_valid_key(&name);
    // Not the name of the branch that is spelled with "_".
    assert_ne!(name, clean("fix_typo"));
    assert_ne!(name, clean("fix;typo"));
}

#[test]
fn non_ascii_bytes_are_replaced() {
    let name = clean("héllo");
    assert_hashed(&name, "h__llo");
    assert!(name.is_ascii());
}

#[test]
fn a_long_name_is_cut_and_hashed() {
    for branch in ["c".repeat(MAX + 1), "d".repeat(600), ",".repeat(1000)] {
        let name = clean(&branch);
        assert_eq!(name.len(), MAX, "{name}");
        assert_valid_key(&name);
    }
    let name = clean(&"d".repeat(600));
    assert_hashed(&name, &"d".repeat(MAX - 17));
    // Two long names that share their first MAX characters differ.
    let same_start = "e".repeat(300);
    assert_ne!(
        clean(&format!("{same_start}1")),
        clean(&format!("{same_start}2"))
    );
}

#[test]
fn no_branch_is_a_usage_error() {
    for args in [&[][..], &[""], &["a", "b"]] {
        Command::new("bash")
            .arg(script())
            .args(args)
            .assert()
            .code(2)
            .stdout("")
            .stderr(contains("usage:"));
    }
}

/// `expr` of the restore step in CI's workflow, by yq.
fn restore_step(expr: &str) -> String {
    let workflow = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml");
    let out = Command::new("yq")
        .arg(format!(
            r#".jobs.check.steps[] | select(.id == "restore") | {expr}"#
        ))
        .arg(workflow)
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn ci_keys_the_cache_by_the_cleaned_branch_name() {
    let branch = "${{ steps.branch.outputs.name }}";
    assert_eq!(
        restore_step(".with.key"),
        format!("greenshift-db-{branch}-${{{{ github.run_id }}}}-${{{{ github.run_attempt }}}}\n")
    );
    // The first restore key is the key's prefix, up to the run id.
    let prefix = format!("greenshift-db-{branch}-");
    assert_eq!(
        restore_step(".with.restore-keys")
            .lines()
            .collect::<Vec<_>>(),
        [prefix.as_str(), "greenshift-db-main-", ""]
    );
    assert_eq!(clean("main"), "main");
}

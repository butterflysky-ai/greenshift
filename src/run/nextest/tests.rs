use super::*;
use crate::tainted::Tainted;

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn backend(args: &[&str]) -> Nextest {
    Nextest::new(&os(args), None, MAX_FILTERSET_BYTES).unwrap()
}

#[test]
fn with_nothing_passed_on_it_uses_its_own_profile() {
    let n = backend(&[]);
    assert_eq!(n.profile, "greenshift");
    assert!(n.list_args.is_empty());
    assert!(n.run_args.is_empty());
}

#[test]
fn ordinary_arguments_go_to_list_and_run() {
    let args = [
        "--workspace",
        "-p",
        "core",
        "--locked",
        "--run-ignored",
        "all",
        "name",
    ];
    let n = backend(&args);
    assert_eq!(n.list_args, os(&args));
    assert_eq!(n.run_args, os(&args));
}

#[test]
fn runner_options_go_to_run_only() {
    let n = backend(&[
        "--no-fail-fast",
        "-j",
        "4",
        "--retries=2",
        "-j8",
        "--locked",
        "--message-format",
        "libtest-json",
        "--nff",
    ]);
    assert_eq!(n.list_args, os(&["--locked"]));
    assert_eq!(
        n.run_args,
        os(&[
            "--no-fail-fast",
            "-j",
            "4",
            "--retries=2",
            "-j8",
            "--locked",
            "--message-format",
            "libtest-json",
            "--nff",
        ])
    );
}

#[test]
fn filters_that_choose_tests_go_to_list_only() {
    // The listing applies them; run gets the tests they chose.
    let n = backend(&[
        "-E",
        "test(a)",
        "--filterset=test(b)",
        "-Etest(c)",
        "--partition",
        "count:1/2",
        "--lib",
    ]);
    assert_eq!(
        n.list_args,
        os(&[
            "-E",
            "test(a)",
            "--filterset=test(b)",
            "-Etest(c)",
            "--partition",
            "count:1/2",
            "--lib",
        ])
    );
    assert_eq!(n.run_args, os(&["--lib"]));
}

#[test]
fn a_profile_passed_on_is_used_instead_of_greenshifts() {
    for args in [
        &["--profile", "ci"][..],
        &["--profile=ci"],
        &["-P", "ci"],
        &["-Pci"],
    ] {
        let n = backend(args);
        assert_eq!(n.profile, "ci", "{args:?}");
        assert!(n.list_args.is_empty(), "{args:?}");
        assert!(n.run_args.is_empty(), "{args:?}");
    }
    assert!(Nextest::new(&os(&["--profile"]), None, 1).is_err());
    assert!(Nextest::new(&os(&["--profile="]), None, 1).is_err());
}

#[test]
fn the_profile_in_nextest_profile_is_used_unless_one_is_passed_on() {
    // nextest's own --profile reads NEXTEST_PROFILE; greenshift always
    // passes --profile, so it must read it in nextest's place.
    let env = |value: &str, args: &[&str]| {
        Nextest::new(&os(args), Some(OsStr::new(value)), MAX_FILTERSET_BYTES)
    };
    assert_eq!(env("ci", &[]).unwrap().profile, "ci");
    assert_eq!(env("ci", &["--profile", "local"]).unwrap().profile, "local");
    // Empty is unset, as clap takes it.
    assert_eq!(env("", &[]).unwrap().profile, PROFILE);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bad = OsStr::from_bytes(b"\xff");
        assert!(Nextest::new(&[], Some(bad), 1).is_err());
    }
}

#[test]
fn everything_after_a_separator_goes_to_both_as_it_is() {
    let n = backend(&["--no-fail-fast", "--", "--exact", "-E", "--no-run"]);
    assert_eq!(n.list_args, os(&["--", "--exact", "-E", "--no-run"]));
    assert_eq!(
        n.run_args,
        os(&["--no-fail-fast", "--", "--exact", "-E", "--no-run"])
    );
}

#[test]
fn similar_names_are_not_taken_for_options() {
    // Not --profile, -E or --no-run: passed on to both.
    let args = ["--profiles", "-p", "--no-runs", "--retries-x"];
    let n = backend(&args);
    assert_eq!(n.profile, "greenshift");
    assert_eq!(n.list_args, os(&args));
    assert_eq!(n.run_args, os(&args));
}

#[test]
fn list_and_run_arguments_put_greenshifts_first() {
    let n = backend(&["--locked", "--no-fail-fast", "--", "--exact"]);
    let config = Path::new("/scratch/list.toml");
    assert_eq!(
        n.list_command(config).unwrap(),
        os(&[
            "nextest",
            "list",
            "--tool-config-file",
            "greenshift:/scratch/list.toml",
            "--profile",
            "greenshift",
            "--message-format",
            "json",
            "--locked",
            "--",
            "--exact",
        ])
    );
    assert_eq!(
        n.run_command(Path::new("/scratch/rich-1.toml"), "test(=a)")
            .unwrap(),
        os(&[
            "nextest",
            "run",
            "--tool-config-file",
            "greenshift:/scratch/rich-1.toml",
            "--profile",
            "greenshift",
            "-E",
            "test(=a)",
            "--locked",
            "--no-fail-fast",
            "--",
            "--exact",
        ])
    );
}

#[test]
fn a_relative_tool_config_path_is_refused() {
    // nextest takes only absolute ones.
    assert!(backend(&[]).list_command(Path::new("list.toml")).is_err());
}

#[test]
fn the_tool_config_sends_the_profiles_junit_to_one_file() {
    assert_eq!(
        backend(&[])
            .tool_config(Path::new("/s/rich-1.xml"))
            .unwrap(),
        "[profile.\"greenshift\".junit]\npath = \"/s/rich-1.xml\"\n"
    );
    assert_eq!(
        backend(&["-P", "c\"i"])
            .tool_config(Path::new("/s/a\\b\n.xml"))
            .unwrap(),
        "[profile.\"c\\\"i\".junit]\npath = \"/s/a\\\\b\\u000A.xml\"\n"
    );
}

fn terms(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("test(=t{i:02})")).collect()
}

#[test]
fn batches_fit_under_the_limit_and_keep_the_terms_in_order() {
    let terms = terms(30);
    // Each term is 10 bytes; " | " is 3.
    for max in [10, 23, 36, 100, 1000] {
        let batches = batches(&terms, max);
        for batch in &batches {
            assert!(batch.len() <= max, "{max}: {batch:?}");
        }
        let union: Vec<&str> = batches.iter().flat_map(|b| b.split(" | ")).collect();
        assert_eq!(union, terms, "{max}");
    }
    assert_eq!(batches(&terms, 1000).len(), 1);
    assert_eq!(batches(&terms, 36).len(), 10);
    assert_eq!(batches(&terms, 35).len(), 15);
}

#[test]
fn a_filterset_over_the_limit_splits_into_several() {
    let terms = terms(3);
    assert_eq!(
        batches(&terms, 23),
        ["test(=t00) | test(=t01)", "test(=t02)"]
    );
    assert_eq!(
        batches(&terms, 36),
        ["test(=t00) | test(=t01) | test(=t02)"]
    );
}

#[test]
fn a_term_longer_than_the_limit_is_a_batch_alone() {
    let terms = vec!["a".to_owned(), "b".repeat(50), "c".to_owned()];
    assert_eq!(batches(&terms, 10), ["a", &"b".repeat(50), "c"]);
    assert!(batches(&[], 10).is_empty());
}

fn test(suite: &str, name: &str) -> InventoryTest {
    InventoryTest {
        suite: Tainted::raw(suite),
        name: Tainted::raw(name),
    }
}

#[test]
fn filtersets_name_each_test_and_none_with_no_tests() {
    let (a, b) = (test("bin", "a, (b)"), test("bin", "c"));
    assert_eq!(
        filtersets(&[&a, &b], 1000),
        [r"(binary_id(=bin) & test(=a\, (b\))) | (binary_id(=bin) & test(=c))"]
    );
    assert_eq!(filtersets(&[&a, &b], 30).len(), 2);
    assert_eq!(filtersets(&[], 1000), ["none()"]);
}

#[test]
fn each_invocation_has_its_own_junit_and_config() {
    let n = Nextest::new(&os(&["--no-fail-fast"]), None, 40).unwrap();
    let tests: Vec<InventoryTest> = ["a", "b", "c"].map(|t| test("bin", t)).into();
    let tests: Vec<&InventoryTest> = tests.iter().collect();
    let dir = Path::new("/scratch");
    let runs = n.invocations(Class::Poor, &tests, dir).unwrap();
    assert_eq!(runs.len(), 3, "one term of 30 bytes fits in 40");
    for (at, run) in runs.iter().enumerate() {
        let stem = format!("poor-{}", at + 1);
        let config = dir.join(format!("{stem}.toml"));
        assert_eq!(run.program, "cargo");
        assert_eq!(run.junit, dir.join(format!("{stem}.xml")));
        assert_eq!(
            run.setup,
            [(config.clone(), n.tool_config(&run.junit).unwrap())]
        );
        let name = ["a", "b", "c"][at];
        assert_eq!(
            run.args,
            n.run_command(&config, &format!("(binary_id(=bin) & test(={name}))"))
                .unwrap()
        );
    }
    let rich = n.invocations(Class::Rich, &tests[..1], dir).unwrap();
    assert_eq!(rich[0].junit, dir.join("rich-1.xml"));
}

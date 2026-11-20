//! Completed JUnit payloads reject mutation, including from their producer parent.

use std::path::Path;
use std::process::Command;

#[test]
fn production_parser_and_construction_preserve_completed_read_and_consume_contract() {
    use greenshift::junit::{parse_str, Outcome, Report, TestCase};
    use greenshift::tainted::Tainted;

    let parsed = parse_str(
        r#"<testsuite name="suite"><testcase name="first" time="0.012"><skipped/><failure/></testcase><testcase name="second"/></testsuite>"#,
    ).unwrap();
    let cases = vec![
        TestCase::new(
            Tainted::raw("suite"),
            Tainted::raw("first"),
            Outcome::Failed,
            Some(12),
        ),
        TestCase::new(
            Tainted::raw("suite"),
            Tainted::raw("second"),
            Outcome::Passed,
            None,
        ),
    ];
    let constructed = Report::new(cases);
    assert_eq!(parsed, constructed);
    let consumed: Vec<_> = parsed.into_cases().collect();
    assert_eq!(consumed, constructed.cases());
    assert_eq!(consumed[0].suite().as_raw(), "suite");
    assert_eq!(consumed[0].name().as_raw(), "first");
    assert_eq!(consumed[0].outcome(), Outcome::Failed);
    assert_eq!(consumed[0].duration_ms(), Some(12));
}

fn fixture(body: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Use the actual completed payload, Outcome and text types at the
    // producer's privacy boundary.
    format!(
        "#![allow(dead_code,unused_mut)]\n\
         #[path={outcome:?}] mod outcome;\n\
         use outcome::Outcome;\n\
         #[path={tainted:?}] mod tainted;\n\
         #[path={payload:?}] mod payload;\n\
         use payload::{{Report,TestCase}};\n\
         use tainted::Tainted;\n\
         fn main() {{\n\
             let mut case=TestCase::new(Tainted::raw(\"suite\"),Tainted::raw(\"name\\n\"),Outcome::Failed,Some(12));\n\
             let mut cases=Vec::new();\n\
             cases.push(case.clone());\n\
             let mut report=Report::new(cases);\n\
             assert_eq!(case.suite().as_raw(),\"suite\");\n\
             assert_eq!(case.name().to_string(),\"name\\\\n\");\n\
             assert_eq!(case.outcome(),Outcome::Failed);\n\
             assert_eq!(case.duration_ms(),Some(12));\n\
             assert_eq!(report.cases(),&[case.clone()]);\n\
             assert_eq!(report.clone().into_cases().collect::<Vec<_>>(),vec![case.clone()]);\n\
             assert!(Report::default().cases().is_empty());\n\
             {body}\n\
         }}\n",
        tainted = root.join("tainted.rs"),
        outcome = root.join("junit/outcome.rs"),
        payload = root.join("junit/payload.rs"),
    )
}

fn compile(body: &str, execute: bool) -> (std::process::Output, Option<std::process::Output>) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("contract.rs");
    let binary = directory.path().join("contract");
    std::fs::write(&source, fixture(body)).unwrap();
    let compilation = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2021")
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    let execution =
        (execute && compilation.status.success()).then(|| Command::new(binary).output().unwrap());
    (compilation, execution)
}

#[test]
fn completed_junit_rejects_mutation_after_valid_construction_read_and_consume_setup() {
    let (positive, execution) = compile("", true);
    assert!(
        positive.status.success(),
        "valid setup: {}",
        String::from_utf8_lossy(&positive.stderr)
    );
    let execution = execution.unwrap();
    assert!(
        execution.status.success(),
        "read/consume contract: {}",
        String::from_utf8_lossy(&execution.stderr)
    );
    for (body, diagnostic) in [
        ("report.cases.clear();", "error[E0616]"),
        ("case.suite=Tainted::raw(\"changed\");", "error[E0616]"),
        ("case.name=Tainted::raw(\"changed\");", "error[E0616]"),
        ("case.outcome=Outcome::Passed;", "error[E0616]"),
        ("case.duration_ms=None;", "error[E0616]"),
        ("report.cases()[0]=case.clone();", "error[E0594]"),
        (
            "case.name().as_raw().make_ascii_uppercase();",
            "error[E0596]",
        ),
        (
            "case.suite().as_raw().make_ascii_uppercase();",
            "error[E0596]",
        ),
        ("report.cases_mut();", "error[E0599]"),
        ("report.into_cases().as_mut_slice();", "error[E0599]"),
        ("case.name_mut();", "error[E0599]"),
    ] {
        let (negative, _) = compile(body, false);
        let error = String::from_utf8_lossy(&negative.stderr);
        assert!(
            !negative.status.success(),
            "mutation unexpectedly compiled: {body}"
        );
        assert!(
            error.contains(diagnostic),
            "wrong failure for {body}: {error}"
        );
        assert!(!error.contains("unresolved import"));
        assert!(!error.contains("couldn't read"));
    }
}

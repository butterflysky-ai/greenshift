//! Producer-side mutation must not bypass completed history payloads.

use std::path::Path;
use std::process::Command;

fn fixture(body: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Compile the actual payloads, builders, Outcome and Tainted implementations
    // together at their producer's privacy boundary.
    format!(
        "#![allow(dead_code,unused_mut,unused_imports)]\n\
         #[path={outcome:?}] mod junit;\n\
         #[path={tainted:?}] mod tainted;\n\
         #[path={data:?}] mod history_data;\n\
         pub use history_data::{{History,StoredRun,StoredResult}};\n\
         #[path={builder:?}] mod history_builder;\n\
         use history_builder::{{HistoryBuilder,StoredRunBuilder}};\n\
         use tainted::Tainted;\n\
         fn main() {{\n\
             let mut result=StoredResult::new(7,junit::Outcome::Failed,Some(12));\n\
             let mut run_builder=StoredRunBuilder {{id:3,source:\"ci\".to_owned(),partial:false,results:vec![]}};\n\
             run_builder.partial=true;\n\
             run_builder.results.push(result);\n\
             let mut run=run_builder.build();\n\
             let mut builder=HistoryBuilder::default();\n\
             builder.tests.insert(7,(Tainted::raw(\"suite\"),Tainted::raw(\"name\\n\")));\n\
             builder.runs.push(run.clone());\n\
             let mut history=builder.build();\n\
             assert_eq!(history.runs(),&[run.clone()]);\n\
             assert_eq!(history.tests()[&7].1.to_string(),\"name\\\\n\");\n\
             assert_eq!(run.id(),3);\n\
             assert_eq!(run.source(),\"ci\");\n\
             assert!(run.partial());\n\
             assert_eq!(run.results(),&[result]);\n\
             assert_eq!(result.test_id(),7);\n\
             assert_eq!(result.outcome(),junit::Outcome::Failed);\n\
             assert_eq!(result.duration_ms(),Some(12));\n\
             let consumed:Vec<_>=history.clone().into_runs().flat_map(StoredRun::into_results).collect();\n\
             assert_eq!(consumed,vec![result]);\n\
             assert!(History::default().runs().is_empty());\n\
             {body}\n\
         }}\n",
        tainted = root.join("tainted.rs"),
        outcome = root.join("junit/outcome.rs"),
        data = root.join("store/history_data.rs"),
        builder = root.join("store/history_builder.rs"),
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
fn completed_history_rejects_mutation_after_valid_builder_read_and_consume_setup() {
    let (positive, execution) = compile("", true);
    assert!(
        positive.status.success(),
        "valid setup: {}",
        String::from_utf8_lossy(&positive.stderr)
    );
    let execution = execution.unwrap();
    assert!(
        execution.status.success(),
        "builder/read/consume contract: {}",
        String::from_utf8_lossy(&execution.stderr)
    );
    for (body, diagnostic) in [
        ("history.runs.clear();", "error[E0616]"),
        ("history.tests.clear();", "error[E0616]"),
        ("run.id=4;", "error[E0616]"),
        ("run.source.clear();", "error[E0616]"),
        ("run.partial=false;", "error[E0616]"),
        ("run.results.clear();", "error[E0616]"),
        ("result.test_id=8;", "error[E0616]"),
        ("result.outcome=junit::Outcome::Passed;", "error[E0616]"),
        ("result.duration_ms=None;", "error[E0616]"),
        ("history.runs()[0]=run.clone();", "error[E0594]"),
        (
            "history.tests().insert(8,(Tainted::raw(\"s\"),Tainted::raw(\"n\")));",
            "error[E0596]",
        ),
        ("run.source().make_ascii_uppercase();", "error[E0596]"),
        ("run.results()[0]=result;", "error[E0594]"),
        ("history.runs_mut();", "error[E0599]"),
        ("history.tests_mut();", "error[E0599]"),
        ("run.results_mut();", "error[E0599]"),
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

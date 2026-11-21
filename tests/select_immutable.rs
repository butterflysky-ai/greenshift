//! Completed selector records and their nested collections reject owned mutation.

use std::path::Path;
mod common;

fn fixture(body: &str, selector: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Use the actual selector, history payload, Outcome and Tainted implementations.
    let source = r#"
#![allow(dead_code, unused_mut, unused_imports)]
#[path=OUTCOME_PATH] mod junit;
#[path=TAINTED_PATH] mod tainted;
#[path=HISTORY_PATH] mod history_data;
mod store { pub use crate::history_data::{History,StoredRun}; }
#[path=SELECT_PATH] mod select;
use tainted::Tainted;
use history_data::{History,StoredRun,StoredResult};
use select::{Analysis,TestInfo,Pick,Split,Unreached,Line,Lines,Class,Measure,Empirical};

fn main() {
    use crate::{select,junit};
    use crate::history_data::StoredResult;
    let mut info=TestInfo::new(Tainted::raw("suite\t"),Tainted::raw("name\n"),96,2,3);
    let mut pick=Pick::new(0,0);
    let mut tests=Vec::new();
    tests.push(info.clone());
    let mut cover=Vec::new();
    cover.push(pick);
    let mut analysis=Analysis::new(tests,cover,1,2);
    assert_eq!(analysis.tests(), &[info.clone()]);
    assert_eq!(analysis.cover(), &[pick]);
    assert_eq!((analysis.rows(),analysis.green()),(1,2));
    assert_eq!((info.median_eighths(),info.passes(),info.results()),(96,2,3));
    assert_eq!(info.median_ms(),12.0);
    assert_eq!((pick.test(),pick.rows_left()),(0,0));
    let mut missed=Unreached::new(Measure::Confidence,99.0,95.0);
    let mut split=Split::new(1,vec![missed]);
    assert_eq!(split.rich(),1);
    assert_eq!(split.unreached(), &[missed]);
    assert_eq!((missed.measure(),missed.wanted(),missed.highest()),(Measure::Confidence,99.0,95.0));
    assert_eq!(missed.to_string(),"confidence 99% not reached; the highest is 95.0%");
    let mut line=Line::new(Class::Rich,100.0,100.0,info.suite(),info.name());
    assert_eq!((line.class(),line.coverage(),line.confidence()),(Class::Rich,100.0,100.0));
    assert_eq!(line.to_string(),"rich\t100.0\t100.0\tsuite\\t\tname\\n");
    // Borrowed text outlives the short-lived Line and Lines owners.
    let suite={let short=line.clone(); short.suite()};
    let name={let short=Lines::new(vec![line.clone()]); short.as_slice()[0].name()};
    assert_eq!(suite,info.suite());
    assert_eq!(name,info.name());
    let mut lines=select::lines(&analysis,&Empirical,split.rich(),0);
    assert_eq!(lines.as_slice(), &[line.clone()]);
    assert_eq!(lines.len(),1);
    assert!(!lines.is_empty());
    assert_eq!(lines.iter().next(),Some(&line));
    let poor=Line::new(Class::Poor,100.0,50.0,info.suite(),info.name());
    let ordered=Lines::new(vec![line.clone(),poor.clone()]);
    assert_eq!(ordered.as_slice(), &[line.clone(),poor.clone()]);
    let mut borrowed=Vec::new();
    for item in &ordered { borrowed.push(item.class()); }
    assert_eq!(borrowed,[Class::Rich,Class::Poor]);
    let mut consumed=ordered.clone().into_lines();
    assert_eq!(consumed.len(),2);
    assert_eq!(consumed.next(),Some(line.clone()));
    assert_eq!(consumed.next_back(),Some(poor));
    assert_eq!(consumed.next(),None);
    drop(consumed);
    assert!(Lines::default().is_empty());
    // Analysis owns its text: dropping the source History cannot invalidate it.
    let produced={
        let history=History::new(vec![StoredRun::new(1,"ci".to_owned(),false,vec![StoredResult::new(7,junit::Outcome::Failed,Some(12))])],[(7,(info.suite().clone(),info.name().clone()))].into());
        select::analyse(&history).unwrap()
    };
    assert_eq!(produced.tests()[0].name(),info.name());
    assert_eq!(produced.cover(), &[pick]);
    assert_eq!(select::lines(&produced,&Empirical,1,0).as_slice()[0].to_string(),"rich\t100.0\t100.0\tsuite\\t\tname\\n");
    BODY
}
"#;
    source
        .replace(
            "OUTCOME_PATH",
            &format!("{:?}", root.join("junit/outcome.rs")),
        )
        .replace("TAINTED_PATH", &format!("{:?}", root.join("tainted.rs")))
        .replace(
            "HISTORY_PATH",
            &format!("{:?}", root.join("store/history_data.rs")),
        )
        .replace("SELECT_PATH", &format!("{selector:?}"))
        .replace("BODY", body)
}

fn relocate_payload(production: &str, payload: &Path) -> String {
    let marker = "mod payload;";
    let matches = production.matches(marker).count();
    assert_eq!(
        matches, 1,
        "select fixture setup: expected exactly one `mod payload;` marker, found {matches}"
    );
    production.replacen(marker, &format!("#[path={payload:?}] mod payload;"), 1)
}

fn compile(body: &str, execute: bool) -> (std::process::Output, Option<std::process::Output>) {
    common::compile(execute, |directory| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let selector = directory.join("select.rs");
        // Keep the real selector's producer privacy boundary, relocating only
        // its child payload path so no copied definitions can mask a regression.
        let production = std::fs::read_to_string(root.join("select.rs")).unwrap();
        let fixture = fixture(body, &selector);
        let (setup, contract) = fixture.split_once("fn main()").unwrap();
        std::fs::write(
            &selector,
            format!(
                "{}\npub fn contract_test(){contract}",
                relocate_payload(&production, &root.join("select/payload.rs"))
            ),
        )
        .unwrap();
        format!("{setup}\nfn main() {{ select::contract_test(); }}")
    })
}

#[test]
fn selector_fixture_rejects_missing_or_duplicate_payload_markers() {
    for (source, count) in [("mod other;", 0), ("mod payload;\nmod payload;", 2)] {
        let failure =
            std::panic::catch_unwind(|| relocate_payload(source, Path::new("payload.rs")))
                .expect_err("ambiguous fixture source must fail before rustc");
        let message = failure
            .downcast_ref::<String>()
            .expect("assertion diagnostic");
        assert!(
            message.contains("select fixture setup: expected exactly one `mod payload;` marker")
        );
        assert!(message.contains(&format!("found {count}")));
    }
    assert_eq!(
        relocate_payload("mod payload;", Path::new("actual.rs")),
        "#[path=\"actual.rs\"] mod payload;"
    );
}

#[test]
fn selector_completed_payloads_reject_mutation_with_valid_read_clone_consume_setup() {
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
        ("analysis.tests.clear();", "error[E0616]"),
        ("analysis.cover.clear();", "error[E0616]"),
        ("analysis.rows=0;", "error[E0616]"),
        ("analysis.green=0;", "error[E0616]"),
        ("info.suite=Tainted::raw(\"other\");", "error[E0616]"),
        ("info.name=Tainted::raw(\"other\");", "error[E0616]"),
        ("info.median_eighths=0;", "error[E0616]"),
        ("info.passes=0;", "error[E0616]"),
        ("info.results=0;", "error[E0616]"),
        ("pick.test=1;", "error[E0616]"),
        ("pick.rows_left=1;", "error[E0616]"),
        ("split.rich=0;", "error[E0616]"),
        ("split.unreached.clear();", "error[E0616]"),
        ("missed.measure=Measure::Coverage;", "error[E0616]"),
        ("missed.wanted=0.0;", "error[E0616]"),
        ("missed.highest=0.0;", "error[E0616]"),
        ("line.class=Class::Poor;", "error[E0616]"),
        ("line.coverage=0.0;", "error[E0616]"),
        ("line.confidence=0.0;", "error[E0616]"),
        ("line.suite=info.name();", "error[E0616]"),
        ("line.name=info.suite();", "error[E0616]"),
        ("lines.lines.clear();", "error[E0616]"),
        ("analysis.tests()[0]=info.clone();", "error[E0594]"),
        ("analysis.cover()[0]=pick;", "error[E0594]"),
        ("split.unreached()[0]=missed;", "error[E0594]"),
        ("lines.as_slice()[0]=line.clone();", "error[E0594]"),
        (
            "info.name().as_raw().make_ascii_uppercase();",
            "error[E0596]",
        ),
        (
            "line.suite().as_raw().make_ascii_uppercase();",
            "error[E0596]",
        ),
        ("lines.into_lines().as_mut_slice();", "error[E0599]"),
        ("lines.iter_mut();", "error[E0599]"),
        ("analysis.tests_mut();", "error[E0599]"),
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

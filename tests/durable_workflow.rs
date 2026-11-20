//! Regression guards for the reference workflow's privilege/failure boundaries.
use serde_json::Value;

fn workflow() -> Value {
    let output = std::process::Command::new("yq")
        .args(["-o=json", ".", "examples/durable-history.github.yml"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("mikefarah yq installed as for cache_guard tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn only_fresh_default_branch_publisher_can_write_and_it_never_checks_out_test_code() {
    let workflow = workflow();
    let test = &workflow["jobs"]["test"];
    let publisher = &workflow["jobs"]["publish"];
    assert_eq!(test["permissions"]["contents"], "read");
    assert_eq!(publisher["permissions"]["contents"], "write");
    assert_eq!(publisher["needs"], "test");
    let condition = publisher["if"].as_str().unwrap();
    assert!(condition.contains("github.event_name == 'push'"));
    assert!(condition.contains("github.event.repository.default_branch"));
    for step in publisher["steps"].as_array().unwrap() {
        assert!(!step["uses"].as_str().unwrap_or("").contains("checkout"));
        assert!(!step["run"].as_str().unwrap_or("").contains("cargo"));
    }
}

#[test]
fn missing_artifact_or_cache_never_fails_workflow_or_attempts_publishing_missing_data() {
    let workflow = workflow();
    let steps = workflow["jobs"]["publish"]["steps"].as_array().unwrap();
    let download = steps.iter().find(|s| s["id"] == "artifact").unwrap();
    assert_eq!(download["continue-on-error"], true);
    let publish = steps.iter().find(|s| s["id"] == "save").unwrap();
    assert!(publish["if"]
        .as_str()
        .unwrap()
        .contains("steps.artifact.outcome == 'success'"));
    let cache = steps.iter().find(|s| s["id"] == "cache").unwrap();
    assert_eq!(cache["continue-on-error"], true);
    assert!(cache["if"]
        .as_str()
        .unwrap()
        .contains("steps.save.outputs.key != ''"));
    assert!(steps.iter().any(|s| s["if"]
        .as_str()
        .unwrap_or("")
        .contains("steps.artifact.outcome == 'failure'")
        && s["run"].as_str().unwrap_or("").contains("::warning::")));
}

#[test]
fn checkout_token_and_cache_wiring_preserve_the_job_trust_boundary() {
    let workflow = workflow();
    let tests = workflow["jobs"]["test"]["steps"].as_array().unwrap();
    let checkout = tests
        .iter()
        .find(|step| {
            step["uses"]
                .as_str()
                .unwrap_or("")
                .starts_with("actions/checkout@")
        })
        .unwrap();
    assert_eq!(checkout["with"]["persist-credentials"], false);
    assert!(workflow["env"]["GREENSHIFT_HISTORY_TOKEN"].is_null());
    for name in ["test", "publish"] {
        let job = &workflow["jobs"][name];
        assert!(job["env"]["GREENSHIFT_HISTORY_TOKEN"].is_null());
        for step in job["steps"].as_array().unwrap() {
            if !step["env"]["GREENSHIFT_HISTORY_TOKEN"].is_null() {
                assert!(step["run"].as_str().unwrap().contains("greenshift"));
                assert_eq!(
                    step["env"]["GREENSHIFT_HISTORY_TOKEN"],
                    "${{ github.token }}"
                );
            }
        }
    }
    let publisher = workflow["jobs"]["publish"]["steps"].as_array().unwrap();
    let save = publisher.iter().find(|step| step["id"] == "save").unwrap();
    assert_eq!(
        save["env"]["GREENSHIFT_HISTORY_TOKEN"],
        "${{ github.token }}"
    );
    for name in ["Resolve canonical cache key", "Run tests and export data"] {
        let step = tests.iter().find(|step| step["name"] == name).unwrap();
        assert_eq!(
            step["env"]["GREENSHIFT_HISTORY_TOKEN"],
            "${{ github.token }}"
        );
    }
    assert!(save["run"]
        .as_str()
        .unwrap()
        .contains("--default-branch \"$DEFAULT_BRANCH\""));
    let cache = tests.iter().find(|step| step["id"] == "cache").unwrap();
    assert_eq!(cache["continue-on-error"], true);
    assert!(tests.iter().any(|step| step["if"]
        .as_str()
        .unwrap_or("")
        .contains("steps.cache.outcome == 'failure'")
        && step["run"].as_str().unwrap_or("").contains("::warning::")));
}

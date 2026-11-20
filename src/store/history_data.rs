//! Completed history records. Mutable assembly lives in `history_builder`.

use std::collections::HashMap;

use crate::junit::Outcome;
use crate::tainted::Tainted;

/// One stored result, as read back by [`super::Store::history`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredResult {
    test_id: i64,
    outcome: Outcome,
    duration_ms: Option<i64>,
}

impl StoredResult {
    /// Construct a completed result using the store's local test identity.
    pub fn new(test_id: i64, outcome: Outcome, duration_ms: Option<i64>) -> Self {
        Self {
            test_id,
            outcome,
            duration_ms,
        }
    }

    /// `tests.id`; [`History::tests`] names it.
    pub fn test_id(&self) -> i64 {
        self.test_id
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn duration_ms(&self) -> Option<i64> {
        self.duration_ms
    }
}

/// One stored run and its results, as read back by [`super::Store::history`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRun {
    id: i64,
    source: String,
    partial: bool,
    results: Vec<StoredResult>,
}

impl StoredRun {
    pub(super) fn new(id: i64, source: String, partial: bool, results: Vec<StoredResult>) -> Self {
        Self {
            id,
            source,
            partial,
            results,
        }
    }

    pub fn id(&self) -> i64 {
        self.id
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    /// Whether the run was recorded as holding only part of the suite.
    pub fn partial(&self) -> bool {
        self.partial
    }
    /// Results in test id order when read by [`super::Store::history`].
    pub fn results(&self) -> &[StoredResult] {
        &self.results
    }
    /// Consume the completed run, yielding its results in their stored order.
    pub fn into_results(self) -> impl ExactSizeIterator<Item = StoredResult> {
        self.results.into_iter()
    }
}

/// The runs and results [`super::Store::history`] read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    runs: Vec<StoredRun>,
    tests: HashMap<i64, (Tainted, Tainted)>,
}

impl History {
    pub(super) fn new(runs: Vec<StoredRun>, tests: HashMap<i64, (Tainted, Tainted)>) -> Self {
        Self { runs, tests }
    }

    /// Runs oldest first, by run id when read by [`super::Store::history`].
    pub fn runs(&self) -> &[StoredRun] {
        &self.runs
    }
    /// Suite and name of every test, by local test id.
    pub fn tests(&self) -> &HashMap<i64, (Tainted, Tainted)> {
        &self.tests
    }
    /// Consume the completed history, yielding runs in their stored order.
    pub fn into_runs(self) -> impl ExactSizeIterator<Item = StoredRun> {
        self.runs.into_iter()
    }
}

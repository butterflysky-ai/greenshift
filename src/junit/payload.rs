//! Completed JUnit values; parser and aggregation state lives outside this module.

use super::Outcome;
use crate::tainted::Tainted;

/// One completed `<testcase>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    suite: Tainted,
    name: Tainted,
    outcome: Outcome,
    duration_ms: Option<i64>,
}

impl TestCase {
    pub fn new(suite: Tainted, name: Tainted, outcome: Outcome, duration_ms: Option<i64>) -> Self {
        Self {
            suite,
            name,
            outcome,
            duration_ms,
        }
    }

    /// `classname`, or the enclosing suite's name when there is none.
    pub fn suite(&self) -> &Tainted {
        &self.suite
    }
    pub fn name(&self) -> &Tainted {
        &self.name
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    /// `time` rounded to whole milliseconds; `None` when the producer gave no time.
    pub fn duration_ms(&self) -> Option<i64> {
        self.duration_ms
    }
}

/// Every test case in one JUnit document, in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    cases: Vec<TestCase>,
}

impl Report {
    pub fn new(cases: Vec<TestCase>) -> Self {
        Self { cases }
    }
    pub fn cases(&self) -> &[TestCase] {
        &self.cases
    }
    pub fn into_cases(self) -> impl DoubleEndedIterator<Item = TestCase> + ExactSizeIterator {
        self.cases.into_iter()
    }
}

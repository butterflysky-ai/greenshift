//! Mutable producers for completed, read-only history payloads.

use std::collections::HashMap;

use super::{History, StoredResult, StoredRun};
use crate::tainted::Tainted;

/// Assemble a run before completing it with [`Self::build`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredRunBuilder {
    pub id: i64,
    pub source: String,
    pub partial: bool,
    pub results: Vec<StoredResult>,
}

impl StoredRunBuilder {
    pub fn build(self) -> StoredRun {
        StoredRun::new(self.id, self.source, self.partial, self.results)
    }
}

/// Assemble named tests and completed runs before completing a history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryBuilder {
    pub runs: Vec<StoredRun>,
    pub tests: HashMap<i64, (Tainted, Tainted)>,
}

impl HistoryBuilder {
    pub fn build(self) -> History {
        History::new(self.runs, self.tests)
    }
}

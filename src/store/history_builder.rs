//! Mutable producers for completed, read-only history payloads.

use super::{StoredResult, StoredRun};

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

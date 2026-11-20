//! Concrete immutable completed snapshot payloads, distinct from load builders.

use std::sync::Arc;

use super::{ResultRow, Run};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct FrozenBytes(Arc<[u8]>);

impl FrozenBytes {
    pub(super) fn copy_from(bytes: &[u8]) -> Self {
        Self(Arc::from(bytes))
    }
    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct FrozenFiles(Box<[(i64, FrozenBytes)]>);

impl FrozenFiles {
    pub(super) fn from_vec(files: Vec<(i64, FrozenBytes)>) -> Self {
        Self(files.into_boxed_slice())
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &(i64, FrozenBytes)> {
        self.0.iter()
    }
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct FrozenResults(Box<[ResultRow]>);

impl FrozenResults {
    pub(super) fn from_vec(results: Vec<ResultRow>) -> Self {
        Self(results.into_boxed_slice())
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &ResultRow> {
        self.0.iter()
    }
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

pub(super) struct FrozenRuns(Box<[Run]>);

impl FrozenRuns {
    pub(super) fn from_vec(runs: Vec<Run>) -> Self {
        Self(runs.into_boxed_slice())
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &Run> {
        self.0.iter()
    }
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

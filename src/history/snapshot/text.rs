//! Snapshot-owned immutable text. Sharing never exposes a mutable Arc handle.

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct FrozenText(Arc<str>);

impl FrozenText {
    pub(super) fn new(text: String) -> Self {
        Self(Arc::from(text))
    }

    pub(super) fn copy_from(text: &str) -> Self {
        Self(Arc::from(text))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

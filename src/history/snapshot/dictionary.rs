//! Mutable dictionary construction is separate from its frozen read-only view.

use std::collections::HashMap;

use super::text::FrozenText;

/// Index into exactly one dictionary, never a durable or cross-snapshot identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TestId(usize);

impl TestId {
    pub(super) fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct TestKey {
    suite: FrozenText,
    name: FrozenText,
}

impl TestKey {
    pub(super) fn new(suite: FrozenText, name: FrozenText) -> Self {
        Self { suite, name }
    }

    pub(super) fn suite(&self) -> &FrozenText {
        &self.suite
    }

    pub(super) fn name(&self) -> &FrozenText {
        &self.name
    }
}

pub(super) struct TestDictionary(Box<[TestKey]>);

impl TestDictionary {
    pub(super) fn get(&self, id: TestId) -> Option<&TestKey> {
        self.0.get(id.index())
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &TestKey> {
        self.0.iter()
    }
}

#[derive(Default)]
pub(super) struct DictionaryBuilder {
    keys: Vec<TestKey>,
    ids: HashMap<TestKey, TestId>,
}

impl DictionaryBuilder {
    pub(super) fn id_for(&self, key: &TestKey) -> Option<TestId> {
        self.ids.get(key).copied()
    }

    pub(super) fn insert(&mut self, key: TestKey) -> TestId {
        if let Some(id) = self.ids.get(&key) {
            return *id;
        }
        let id = TestId(self.keys.len());
        self.keys.push(key.clone());
        self.ids.insert(key, id);
        id
    }

    pub(super) fn freeze(self) -> TestDictionary {
        TestDictionary(self.keys.into_boxed_slice())
    }
}

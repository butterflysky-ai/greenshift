use std::collections::{BTreeMap, BTreeSet};

/// Completed binary identities in sorted order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinaryIds {
    ids: BTreeSet<String>,
}

impl BinaryIds {
    pub fn new(ids: BTreeSet<String>) -> Self {
        Self { ids }
    }
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }
    pub fn iter(&self) -> std::collections::btree_set::Iter<'_, String> {
        self.ids.iter()
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn into_ids(self) -> impl DoubleEndedIterator<Item = String> + ExactSizeIterator {
        self.ids.into_iter()
    }
}

impl<'a> IntoIterator for &'a BinaryIds {
    type Item = &'a String;
    type IntoIter = std::collections::btree_set::Iter<'a, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// What a nextest listing says exists now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    binaries: BinaryIds,
    tests: BTreeMap<String, BTreeSet<String>>,
}

impl Listing {
    pub fn new(binaries: BinaryIds, tests: BTreeMap<String, BTreeSet<String>>) -> Self {
        Self { binaries, tests }
    }
    pub fn binaries(&self) -> &BinaryIds {
        &self.binaries
    }
    /// The test names of each binary whose tests the listing gives: the
    /// `testcases` of a full listing's `listed` binaries. A binaries-only
    /// listing, or a binary a full listing skipped, names no tests.
    pub fn tests(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.tests
    }
    pub fn into_binary_ids(self) -> BinaryIds {
        self.binaries
    }
    /// Whether the stored test `suite`, `name` is in the listing: its
    /// binary is, and so is its name if the binary's tests are listed. A
    /// test with no suite matches by name in any binary, so it is kept.
    pub fn has(&self, suite: &str, name: &str) -> bool {
        if suite.is_empty() {
            return true;
        }
        match self.tests.get(suite) {
            Some(names) => names.contains(name),
            None => self.binaries.contains(suite),
        }
    }
}

/// Mutable assembly state used only while parsing a listing.
#[derive(Default)]
pub(super) struct ListingBuilder {
    pub(super) binaries: BTreeSet<String>,
    pub(super) tests: BTreeMap<String, BTreeSet<String>>,
}

impl ListingBuilder {
    pub(super) fn finish(self) -> Listing {
        Listing::new(BinaryIds::new(self.binaries), self.tests)
    }
}

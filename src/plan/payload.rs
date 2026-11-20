use crate::tainted::Tainted;

/// A completed inventory identity, matched by suite and name rather than store id.
/// Text from outside the program remains [`Tainted`], with raw text reachable
/// only through [`Tainted::as_raw`].
///
/// ```
/// let tests = greenshift::plan::parse_inventory(r#"{"suite":"s","name":"n"}"#).unwrap();
/// assert_eq!(tests[0].name().as_raw(), "n");
/// ```
///
/// ```compile_fail
/// let tests = greenshift::plan::parse_inventory(r#"{"suite":"s","name":"n"}"#).unwrap();
/// let name: &str = tests[0].name();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryTest {
    suite: Tainted,
    name: Tainted,
}

impl InventoryTest {
    pub fn new(suite: Tainted, name: Tainted) -> Self {
        Self { suite, name }
    }
    pub fn suite(&self) -> &Tainted {
        &self.suite
    }
    pub fn name(&self) -> &Tainted {
        &self.name
    }
}

/// Completed tests in inventory order, including any repeated identities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    tests: Vec<InventoryTest>,
}

impl Inventory {
    pub fn new(tests: Vec<InventoryTest>) -> Self {
        Self { tests }
    }
    pub fn as_slice(&self) -> &[InventoryTest] {
        &self.tests
    }
    pub fn iter(&self) -> std::slice::Iter<'_, InventoryTest> {
        self.tests.iter()
    }
    pub fn len(&self) -> usize {
        self.tests.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }
    pub fn into_tests(self) -> impl DoubleEndedIterator<Item = InventoryTest> + ExactSizeIterator {
        self.tests.into_iter()
    }
}

impl std::ops::Deref for Inventory {
    type Target = [InventoryTest];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a Inventory {
    type Item = &'a InventoryTest;
    type IntoIter = std::slice::Iter<'a, InventoryTest>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Completed rich and poor groups borrowing their inventory identities.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan<'a> {
    rich: Vec<&'a InventoryTest>,
    poor: Vec<&'a InventoryTest>,
}

impl<'a> Plan<'a> {
    pub fn new(rich: Vec<&'a InventoryTest>, poor: Vec<&'a InventoryTest>) -> Self {
        Self { rich, poor }
    }
    /// Pick order followed by the remaining rich tests in inventory order.
    pub fn rich(&self) -> &[&'a InventoryTest] {
        &self.rich
    }
    /// Poor tests in selection order.
    pub fn poor(&self) -> &[&'a InventoryTest] {
        &self.poor
    }
}

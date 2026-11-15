//! A runner-neutral plan: what `greenshift plan` prints.
//!
//! Given the tests that exist now (an inventory) and `select`'s lines, the
//! plan splits the inventory into rich tests, to run first, and poor
//! tests, to run after. Every inventory test is in exactly one of them.
//! Poor is exactly the inventory tests on `select`'s poor lines; rich is
//! every other inventory test, so tests new to the store, or seen too few
//! times to be poor, run rich. Stored tests not in the inventory are left
//! out.
//!
//! The inventory is JSON lines, one `{"suite": "...", "name": "..."}`
//! object per test; the plan is one JSON object with `rich` and `poor`
//! arrays of such objects. Names are compared and written raw, as stored.

use std::collections::BTreeMap;

use crate::select::{Class, Line};

/// A test, as the inventory gives it and the plan writes it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TestId {
    pub suite: String,
    pub name: String,
}

/// Why an inventory cannot be read: the 1-based line and what is wrong.
#[derive(Debug, thiserror::Error)]
#[error("not a greenshift inventory: line {line}: {why}")]
pub struct InventoryError {
    line: usize,
    why: String,
}

/// The tests of an inventory: JSON lines, each an object with string
/// `suite` and `name` members (other members are ignored). Blank lines
/// are skipped.
pub fn parse_inventory(text: &str) -> Result<Vec<TestId>, InventoryError> {
    let mut tests = Vec::new();
    for (at, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let error = |why: String| InventoryError { line: at + 1, why };
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|err| error(err.to_string()))?;
        let member = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| error(format!("no string {key:?}")))
        };
        if !value.is_object() {
            return Err(error("not an object".to_owned()));
        }
        tests.push(TestId {
            suite: member("suite")?,
            name: member("name")?,
        });
    }
    Ok(tests)
}

/// The inventory split into rich and poor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan<'a> {
    /// `select`'s rich tests in pick order, then every other non-poor
    /// inventory test in inventory order.
    pub rich: Vec<&'a TestId>,
    /// `select`'s poor tests in its order.
    pub poor: Vec<&'a TestId>,
}

/// Split `inventory` by `select`'s `lines`: poor is the inventory tests on
/// poor lines, rich the rest, rich lines' tests first. A test listed twice
/// is planned once. With no lines (no history) every test is rich.
pub fn plan<'a>(lines: &[Line<'_>], inventory: &'a [TestId]) -> Plan<'a> {
    let key = |t: &'a TestId| (t.suite.as_str(), t.name.as_str());
    let mut unplanned: BTreeMap<(&str, &str), &'a TestId> = BTreeMap::new();
    for test in inventory.iter().rev() {
        unplanned.insert(key(test), test);
    }
    let mut take = |class: Class| -> Vec<&'a TestId> {
        lines
            .iter()
            .filter(|line| line.class == class)
            .filter_map(|line| unplanned.remove(&(line.suite.as_raw(), line.name.as_raw())))
            .collect()
    };
    let mut rich = take(Class::Rich);
    let poor = take(Class::Poor);
    rich.extend(
        inventory
            .iter()
            .filter(|&test| unplanned.remove(&key(test)).is_some()),
    );
    Plan { rich, poor }
}

impl Plan<'_> {
    /// `{"rich":[{"suite":...,"name":...},...],"poor":[...]}` on one line,
    /// members in that order. Strings are JSON-escaped, control characters
    /// included, so the plan is one line.
    pub fn to_json(&self) -> String {
        let group = |tests: &[&TestId]| {
            let tests: Vec<String> = tests
                .iter()
                .map(|t| {
                    format!(
                        r#"{{"suite":{},"name":{}}}"#,
                        serde_json::Value::from(t.suite.as_str()),
                        serde_json::Value::from(t.name.as_str())
                    )
                })
                .collect();
            format!("[{}]", tests.join(","))
        };
        format!(
            r#"{{"rich":{},"poor":{}}}"#,
            group(&self.rich),
            group(&self.poor)
        )
    }
}

#[cfg(test)]
mod tests;

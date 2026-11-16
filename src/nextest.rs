//! Turning a selection into a cargo-nextest filterset: what
//! `greenshift nextest-filter` prints.
//!
//! In nextest's JUnit, a testcase's `classname` is the test binary's id and
//! its `name` the test's name, so a stored test is the nextest test
//! `binary_id(=<suite>) & test(=<name>)`.
//!
//! nextest refuses a whole filterset when any `binary_id(=...)` in it
//! matches no binary it knows, so terms are kept only for binaries in the
//! current inventory: `cargo nextest list --message-format json`. The
//! selection itself is made from the tests the inventory has (see
//! [`Listing::has`]): a gone binary's tests, and a full listing's gone
//! tests, are never rich. A binaries-only listing names no tests, so a
//! test gone from a binary it lists can still be.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::plan::InventoryTest;
use crate::select::{Class, Line};
use crate::tainted::Tainted;

/// `text` as the string of a filterset matcher such as `test(=...)`.
///
/// nextest reads a matcher string up to the first unescaped `)` or `,`,
/// and `\` starts an escape: `\)`, `\,`, `\\`, `\/`, `\n`, `\r`, `\t`,
/// `\b`, `\f` and `\u{HEX}`. So `)`, `,` and `\` are escaped, and `/` too,
/// as nextest writes it; control characters are escaped so the filterset
/// stays on one line. Everything else, spaces included, stands for itself.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ')' | ',' | '\\' | '/' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if c.is_control() => {
                let _ = write!(out, r"\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// The filterset term matching exactly the stored test `suite`, `name`.
/// nextest's JUnit always gives a suite; only other JUnit can lack one.
/// With none, the term matches the name in every binary: an empty
/// `binary_id(=)` is not a valid filterset.
pub fn term(suite: &str, name: &str) -> String {
    if suite.is_empty() {
        format!("test(={})", escape(name))
    } else {
        format!("(binary_id(={}) & test(={}))", escape(suite), escape(name))
    }
}

/// A filterset matching exactly `tests`, as `(suite, name)` pairs:
/// `none()` when there are none.
pub fn any_of<'a>(tests: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let terms: Vec<String> = tests
        .into_iter()
        .map(|(suite, name)| term(suite, name))
        .collect();
    if terms.is_empty() {
        "none()".to_owned()
    } else {
        terms.join(" | ")
    }
}

/// A filterset matching every test except `tests`: `all()` when there are
/// none.
pub fn all_but<'a>(tests: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut tests = tests.into_iter().peekable();
    if tests.peek().is_none() {
        "all()".to_owned()
    } else {
        format!("not ({})", any_of(tests))
    }
}

/// Why a file is not a `cargo nextest list --message-format json` listing.
#[derive(Debug, thiserror::Error)]
#[error("not cargo nextest list JSON: {0}")]
pub struct InventoryError(String);

/// What a `cargo nextest list --message-format json` listing says exists
/// now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// Every binary id.
    pub binaries: BTreeSet<String>,
    /// The test names of each binary whose tests the listing gives: the
    /// `testcases` of a full listing's `listed` binaries. A binaries-only
    /// listing, or a binary a full listing skipped, names no tests.
    pub tests: BTreeMap<String, BTreeSet<String>>,
}

impl Listing {
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

/// A `cargo nextest list --message-format json` listing, full
/// (`rust-suites`) or `--list-type binaries-only` (`rust-binaries`):
/// each entry's `binary-id` and, when it has them, its `testcases`.
pub fn listing(json: &str) -> Result<Listing, InventoryError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|err| InventoryError(err.to_string()))?;
    let entries = ["rust-suites", "rust-binaries"]
        .iter()
        .find_map(|key| value.get(key))
        .ok_or_else(|| InventoryError("no rust-suites or rust-binaries".to_owned()))?
        .as_object()
        .ok_or_else(|| InventoryError("binaries are not an object".to_owned()))?;
    let mut listing = Listing::default();
    for entry in entries.values() {
        let id = entry
            .get("binary-id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| InventoryError("a binary without a binary-id".to_owned()))?;
        // A binary a full listing filters out is `skipped`, with no
        // testcases: its tests are not known, so all of them are kept.
        let listed = entry.get("status").and_then(serde_json::Value::as_str) == Some("listed");
        if let Some(cases) = entry.get("testcases").filter(|_| listed) {
            let cases = cases
                .as_object()
                .ok_or_else(|| InventoryError("testcases are not an object".to_owned()))?;
            listing
                .tests
                .insert(id.to_owned(), cases.keys().cloned().collect());
        }
        listing.binaries.insert(id.to_owned());
    }
    Ok(listing)
}

/// The tests a full `cargo nextest list --message-format json` listing
/// selects: the testcases of each `listed` binary whose `filter-match`
/// status is `matches`, by binary id and then name. A testcase that its
/// filters, or its being ignored, leave out is `mismatch`. Each test's
/// suite is its binary id, as in nextest's JUnit. A binaries-only listing
/// names no tests, so it is refused.
pub fn selected_tests(json: &str) -> Result<Vec<InventoryTest>, InventoryError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|err| InventoryError(err.to_string()))?;
    let entries = value
        .get("rust-suites")
        .ok_or_else(|| InventoryError("no rust-suites: not a full listing".to_owned()))?
        .as_object()
        .ok_or_else(|| InventoryError("binaries are not an object".to_owned()))?;
    let mut tests = Vec::new();
    for entry in entries.values() {
        let id = entry
            .get("binary-id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| InventoryError("a binary without a binary-id".to_owned()))?;
        if entry.get("status").and_then(serde_json::Value::as_str) != Some("listed") {
            continue;
        }
        let cases = entry
            .get("testcases")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| InventoryError("testcases are not an object".to_owned()))?;
        for (name, case) in cases {
            let status = case
                .pointer("/filter-match/status")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| InventoryError("a testcase without a filter-match".to_owned()))?;
            if status == "matches" {
                tests.push(InventoryTest {
                    suite: Tainted::raw(id),
                    name: Tainted::raw(name.as_str()),
                });
            }
        }
    }
    Ok(tests)
}

/// The binary ids of a [`listing`].
pub fn binary_ids(json: &str) -> Result<BTreeSet<String>, InventoryError> {
    listing(json).map(|listing| listing.binaries)
}

/// The filterset for `class` from `select`'s `lines`. Both are decided by
/// the poor lines alone: poor is exactly them, and rich is every other
/// test, so tests new to the store, or not in it at all, run with rich.
/// Poor lines whose suite is not one of `binaries` are left out: those
/// tests are not in the inventory, and their terms would make nextest
/// refuse the filterset. A line with no suite has no `binary_id` term and
/// is kept.
pub fn filter(lines: &[Line<'_>], class: Class, binaries: &BTreeSet<String>) -> String {
    let poor = lines
        .iter()
        .filter(|line| line.class == Class::Poor)
        .filter(|line| {
            let suite = line.suite.as_raw();
            suite.is_empty() || binaries.contains(suite)
        })
        .map(|line| (line.suite.as_raw(), line.name.as_raw()));
    match class {
        Class::Rich => all_but(poor),
        Class::Poor => any_of(poor),
    }
}

#[cfg(test)]
mod tests;

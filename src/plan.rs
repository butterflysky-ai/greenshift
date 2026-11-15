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
//! object per test; the plan is one JSON object with a `history` state
//! (see [`HistoryState`]) and `rich` and `poor` arrays of such objects.
//! Names are compared and written raw, as stored.

use std::collections::HashMap;
use std::fmt::Write;

use crate::select::{Class, Line};
use crate::tainted::Tainted;

/// A test, as the inventory gives it and the plan writes it. Suite and
/// name are text from outside the program, so they are [`Tainted`]: the
/// raw text is reachable only through [`Tainted::as_raw`]. It is matched
/// with the store by suite and name, never by the store's test id.
///
/// ```
/// let tests = greenshift::plan::parse_inventory(r#"{"suite":"s","name":"n"}"#).unwrap();
/// assert_eq!(tests[0].name.as_raw(), "n");
/// ```
///
/// ```compile_fail
/// let tests = greenshift::plan::parse_inventory(r#"{"suite":"s","name":"n"}"#).unwrap();
/// let name: &str = &tests[0].name;
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct InventoryTest {
    pub suite: Tainted,
    pub name: Tainted,
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
pub fn parse_inventory(text: &str) -> Result<Vec<InventoryTest>, InventoryError> {
    let mut tests = Vec::new();
    for (at, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let error = |why: String| InventoryError { line: at + 1, why };
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|err| error(json_why(&err)))?;
        let Some(object) = value.as_object() else {
            return Err(error("not an object".to_owned()));
        };
        let member = |key: &str| {
            object
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(Tainted::raw)
                .ok_or_else(|| error(format!("no string {key:?}")))
        };
        tests.push(InventoryTest {
            suite: member("suite")?,
            name: member("name")?,
        });
    }
    Ok(tests)
}

/// serde_json's message for an error in one inventory line, which it
/// places at line 1 of the line: the column only, as the line number is
/// the inventory's.
fn json_why(err: &serde_json::Error) -> String {
    let full = err.to_string();
    let at = format!(" at line {} column {}", err.line(), err.column());
    match full.strip_suffix(&at) {
        Some(what) => format!("{what} at column {}", err.column()),
        None => full,
    }
}

/// The inventory split into rich and poor.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan<'a> {
    /// `select`'s rich tests in pick order, then every other non-poor
    /// inventory test in inventory order.
    pub rich: Vec<&'a InventoryTest>,
    /// `select`'s poor tests in its order.
    pub poor: Vec<&'a InventoryTest>,
}

/// Split `inventory` by `select`'s `lines`: poor is the inventory tests on
/// poor lines, rich the rest, rich lines' tests first. A test listed twice
/// is planned once. With no lines (no history) every test is rich.
pub fn plan<'a>(lines: &[Line<'_>], inventory: &'a [InventoryTest]) -> Plan<'a> {
    let key = |t: &'a InventoryTest| (&t.suite, &t.name);
    // A test listed twice has one key. The last pass below, not this map,
    // puts the rest in inventory order. Keyed as the store keys a test:
    // by its suite and name, as `Tainted`.
    let mut unplanned: HashMap<(&Tainted, &Tainted), &'a InventoryTest> =
        inventory.iter().map(|test| (key(test), test)).collect();
    let mut take = |class: Class| -> Vec<&'a InventoryTest> {
        lines
            .iter()
            .filter(|line| line.class == class)
            .filter_map(|line| unplanned.remove(&(line.suite, line.name)))
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

/// `text` as a JSON string that reads back as exactly `text`, with every
/// control and invisible character escaped: serde_json escapes C0
/// itself, and DEL, C1 and every [`invisible`] character, which it would
/// leave raw, become `\uXXXX` here (a surrogate pair beyond the BMP), as
/// [`crate::nextest::escape`] escapes control characters for a filterset.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    // serde_json's own escapes are printable ASCII, so this pass leaves
    // them whole and touches only the characters it left raw.
    for c in serde_json::Value::from(text).to_string().chars() {
        if c.is_control() || invisible(c) {
            for unit in c.encode_utf16(&mut [0; 2]) {
                let _ = write!(out, r"\u{unit:04x}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether `c` does not print, by the tables `str::escape_debug` uses
/// (as [`Tainted`]'s display does): it escapes `c` as `\u{…}` in text:
/// a format character (Unicode Cf: bidi controls, U+200B, U+FEFF and
/// so on), a line or paragraph separator (U+2028, U+2029), a space other
/// than U+0020, a private-use or unassigned code point, or a control.
fn invisible(c: char) -> bool {
    // `str::escape_debug` escapes a combining mark only first in the
    // text, so `c` follows another character.
    let text: String = ['a', c].into_iter().collect();
    text.escape_debug().skip(1).take(2).eq(['\\', 'u'])
}

/// Whether the plan could use the history store: the plan's `history`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryState {
    /// The store was read (with or without runs) or was just created
    /// empty.
    Ok,
    /// The store could not be created, read or used, so the plan was made
    /// with no history: every test is rich.
    Error,
}

impl HistoryState {
    /// The state as the plan writes it: `ok` or `error`.
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryState::Ok => "ok",
            HistoryState::Error => "error",
        }
    }
}

impl Plan<'_> {
    /// `{"history":"ok","rich":[{"suite":...,"name":...},...],"poor":[...]}`
    /// on one line, members in that order: `history` comes first, as it
    /// says how to read the groups. Strings are JSON-escaped, every control
    /// and invisible character included, so the plan is one line and no
    /// such character reaches a terminal raw.
    pub fn to_json(&self, history: HistoryState) -> String {
        let group = |tests: &[&InventoryTest]| {
            let tests: Vec<String> = tests
                .iter()
                .map(|t| {
                    format!(
                        r#"{{"suite":{},"name":{}}}"#,
                        json_string(t.suite.as_raw()),
                        json_string(t.name.as_raw())
                    )
                })
                .collect();
            format!("[{}]", tests.join(","))
        };
        format!(
            r#"{{"history":"{}","rich":{},"poor":{}}}"#,
            history.as_str(),
            group(&self.rich),
            group(&self.poor)
        )
    }
}

#[cfg(test)]
mod tests;

//! Turning a selection into a cargo-nextest filterset: what
//! `greenshift nextest-filter` prints.
//!
//! In nextest's JUnit, a testcase's `classname` is the test binary's id and
//! its `name` the test's name, so a stored test is the nextest test
//! `binary_id(=<suite>) & test(=<name>)`.

use std::fmt::Write;

use crate::select::{Class, Line};

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
/// With no suite, which nextest's JUnit always gives, it matches the name
/// in every binary: an empty `binary_id(=)` is not a valid filterset.
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

/// The filterset for `class` from `select`'s `lines`. Both are decided by
/// the poor lines alone: poor is exactly them, and rich is every other
/// test, so tests new to the store, or not in it at all, run with rich.
pub fn filter(lines: &[Line<'_>], class: Class) -> String {
    let poor = lines
        .iter()
        .filter(|line| line.class == Class::Poor)
        .map(|line| (line.suite.as_raw(), line.name.as_raw()));
    match class {
        Class::Rich => all_but(poor),
        Class::Poor => any_of(poor),
    }
}

#[cfg(test)]
mod tests;

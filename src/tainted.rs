//! Text from outside the program that may hold terminal control bytes.
//!
//! Test and suite names and other text read from input files, SQLite's
//! messages about the store, and paths and rejected values given on the
//! command line are shown through [`Tainted`], which prints
//! escaped, so a terminal escape sequence or newline in them cannot reach
//! the user's terminal raw.

use std::fmt;
use std::path::Path;

/// Escape text quoted from a file as `{:?}` would, except that quotes are
/// left alone, so it can be shown in an error message.
fn escape_file_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' | '\'' => out.push(c),
            _ => out.extend(c.escape_debug()),
        }
    }
    out
}

/// Text from outside the program, such as text read from an input file or
/// a path given on the command line, which may hold terminal control bytes.
///
/// `Display` writes it escaped: control and invisible characters and
/// backslash as `{:?}` would show them (an ESC
/// prints as `\u{1b}`), quotes left alone. `Debug` escapes too. There is no
/// `Deref<Target = str>` or `AsRef<str>`, so it cannot be passed where a
/// `&str` is expected and printed raw by accident:
///
/// ```compile_fail
/// let name = greenshift::tainted::Tainted::raw("\u{1b}[31mevil");
/// let text: &str = &name;
/// ```
///
/// ```compile_fail
/// let name = greenshift::tainted::Tainted::raw("\u{1b}[31mevil");
/// println!("{}", name.0);
/// ```
///
/// Text is wrapped only through [`Tainted::raw`] or `From<&Path>`, never
/// by `.into()` or `Tainted::from` on a string, so wrapping is always
/// explicit at the call site:
///
/// ```compile_fail
/// let name: greenshift::tainted::Tainted = String::from("evil").into();
/// ```
///
/// ```compile_fail
/// let name = greenshift::tainted::Tainted::from("evil");
/// ```
///
/// ```compile_fail
/// let name = greenshift::tainted::Tainted::raw("evil");
/// let again = greenshift::tainted::Tainted::raw(name);
/// ```
///
/// The text itself is reachable only through [`Tainted::as_raw`], for
/// where it goes to the store or is parsed, not to a terminal:
///
/// ```
/// let name = greenshift::tainted::Tainted::raw("\u{1b}[31mevil");
/// assert_eq!(name.to_string(), r"\u{1b}[31mevil");
/// assert_eq!(name.as_raw(), "\u{1b}[31mevil");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Tainted(String);

impl Tainted {
    /// Wrap text exactly as it came from outside the program, before
    /// anything has escaped it. There is no `From<String>` or
    /// `From<&str>`, so every wrapping is named and can be found by
    /// searching for `Tainted::raw`. Never pass it shown text, such as a
    /// `Tainted` formatted with `{}`: that would be escaped twice.
    pub fn raw(text: impl Into<String>) -> Tainted {
        Tainted(text.into())
    }

    /// The text as given, control bytes and all: unchanged for text read
    /// from an input file, and for a path its lossy UTF-8 form.
    /// Never print it: format the `Tainted` itself instead.
    pub fn as_raw(&self) -> &str {
        &self.0
    }
}

impl From<&Path> for Tainted {
    /// The path as [`Path::display`] shows it (bytes that are not UTF-8
    /// become U+FFFD), so that printing it escapes control characters:
    /// a path from argv is as untrusted as text from a file.
    fn from(path: &Path) -> Tainted {
        Tainted(path.to_string_lossy().into_owned())
    }
}

impl fmt::Display for Tainted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&escape_file_text(&self.0))
    }
}

#[cfg(test)]
mod tests;

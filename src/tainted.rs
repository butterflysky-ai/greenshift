//! Text from outside the program that may hold terminal control bytes.
//!
//! Test and suite names and other text read from input files, and paths
//! given on the command line, are kept as [`Tainted`], which prints
//! escaped, so a terminal escape sequence or newline in them cannot reach
//! the user's terminal raw.

use std::fmt;
use std::path::Path;

/// Escape text quoted from a file as `{:?}` would, except that quotes are
/// left alone, so it can be shown in an error message.
pub(crate) fn escape_file_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' | '\'' => out.push(c),
            _ => out.extend(c.escape_debug()),
        }
    }
    out
}

/// Text read from an input file, or a path given on the command line,
/// which may hold terminal control bytes.
///
/// `Display` writes it escaped: control and invisible characters and
/// backslash as `{:?}` would show them (an ESC
/// prints as `\u{1b}`), quotes left alone. `Debug` escapes too. There is no
/// `Deref<Target = str>` or `AsRef<str>`, so it cannot be passed where a
/// `&str` is expected and printed raw by accident:
///
/// ```compile_fail
/// let name = greenshift::tainted::Tainted::from("\u{1b}[31mevil");
/// let text: &str = &name;
/// ```
///
/// ```compile_fail
/// let name = greenshift::tainted::Tainted::from("\u{1b}[31mevil");
/// println!("{}", name.0);
/// ```
///
/// The text itself is reachable only through [`Tainted::as_raw`], for
/// where it goes to the store or is parsed, not to a terminal:
///
/// ```
/// let name = greenshift::tainted::Tainted::from("\u{1b}[31mevil");
/// assert_eq!(name.to_string(), r"\u{1b}[31mevil");
/// assert_eq!(name.as_raw(), "\u{1b}[31mevil");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Tainted(String);

impl Tainted {
    /// The text as given, control bytes and all: unchanged for text read
    /// from an input file, and for a path its lossy UTF-8 form.
    /// Never print it: format the `Tainted` itself instead.
    pub fn as_raw(&self) -> &str {
        &self.0
    }
}

impl From<String> for Tainted {
    fn from(text: String) -> Tainted {
        Tainted(text)
    }
}

impl From<&str> for Tainted {
    fn from(text: &str) -> Tainted {
        Tainted(text.to_owned())
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

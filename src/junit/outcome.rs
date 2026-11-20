//! The outcome of one test case, shared by parsing and history data.

use std::fmt;

/// How a single test case ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Passed,
    Failed,
    Error,
    Skipped,
}

impl Outcome {
    /// The text stored for this outcome.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Error => "error",
            Outcome::Skipped => "skipped",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

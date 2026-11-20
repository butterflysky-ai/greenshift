//! Completed selector values, separate from mutable algorithm working state.
use super::{Class, Measure};
use crate::tainted::Tainted;
use std::fmt;

/// One universe test and what history says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestInfo {
    suite: Tainted,
    name: Tainted,
    median_eighths: u128,
    passes: u64,
    results: u64,
}

impl TestInfo {
    /// The median duration in milliseconds.
    pub fn median_ms(&self) -> f64 {
        self.median_eighths as f64 / 8.0
    }
}

/// One step of the greedy cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    test: usize,
    rows_left: u64,
}

/// Everything [`super::lines`] needs, computed from history before anything is
/// decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    tests: Vec<TestInfo>,
    cover: Vec<Pick>,
    rows: u64,
    green: u64,
}

/// A threshold no prefix of the cover reached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unreached {
    measure: Measure,
    wanted: f64,
    highest: f64,
}

impl fmt::Display for Unreached {
    /// `<measure> <wanted>% not reached; the highest is <highest>%`, the
    /// highest to one decimal, or to as many more as it takes to show it
    /// below the threshold.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}% not reached; the highest is ",
            self.measure, self.wanted
        )?;
        let below = |d: usize| {
            format!("{:.d$}", self.highest)
                .parse::<f64>()
                .is_ok_and(|shown| shown < self.wanted)
        };
        match (1..=6).find(|&d| below(d)) {
            Some(d) => write!(f, "{:.d$}%", self.highest),
            // Shortest exact form, which is below the threshold.
            None => write!(f, "{}%", self.highest),
        }
    }
}

/// How much of the cover is rich.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    rich: usize,
    unreached: Vec<Unreached>,
}

/// One line of `select` output.
#[derive(Debug, Clone, PartialEq)]
pub struct Line<'a> {
    class: Class,
    coverage: f64,
    confidence: f64,
    suite: &'a Tainted,
    name: &'a Tainted,
}

impl fmt::Display for Line<'_> {
    /// `class<TAB>coverage<TAB>confidence<TAB>suite<TAB>name`. Suite and
    /// name are escaped as [`Tainted`] displays them, so a tab or newline
    /// in either cannot break the line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class = match self.class {
            Class::Rich => "rich",
            Class::Poor => "poor",
        };
        write!(
            f,
            "{class}\t{:.1}\t{:.1}\t{}\t{}",
            self.coverage, self.confidence, self.suite, self.name
        )
    }
}

impl TestInfo {
    pub fn new(
        suite: Tainted,
        name: Tainted,
        median_eighths: u128,
        passes: u64,
        results: u64,
    ) -> Self {
        Self {
            suite,
            name,
            median_eighths,
            passes,
            results,
        }
    }
    pub fn suite(&self) -> &Tainted {
        &self.suite
    }
    pub fn name(&self) -> &Tainted {
        &self.name
    }
    /// Median duration in eighths of a millisecond, so medians of medians
    /// stay exact; see [`TestInfo::median_ms`].
    pub fn median_eighths(&self) -> u128 {
        self.median_eighths
    }
    /// Passed results in full runs.
    pub fn passes(&self) -> u64 {
        self.passes
    }
    /// Passed, failed and error results (not skips) in any run read.
    pub fn results(&self) -> u64 {
        self.results
    }
}
impl Pick {
    pub fn new(test: usize, rows_left: u64) -> Self {
        Self { test, rows_left }
    }
    /// Index into [`Analysis::tests`].
    pub fn test(&self) -> usize {
        self.test
    }
    /// Matrix rows that no test picked so far, this one included, fails in.
    pub fn rows_left(&self) -> u64 {
        self.rows_left
    }
}
impl Analysis {
    pub fn new(tests: Vec<TestInfo>, cover: Vec<Pick>, rows: u64, green: u64) -> Self {
        Self {
            tests,
            cover,
            rows,
            green,
        }
    }
    /// The universe, by suite and then name.
    pub fn tests(&self) -> &[TestInfo] {
        &self.tests
    }
    /// The greedy cover, in pick order. It always ends with no rows left.
    pub fn cover(&self) -> &[Pick] {
        &self.cover
    }
    /// Matrix rows: full runs in which a universe test failed.
    pub fn rows(&self) -> u64 {
        self.rows
    }
    /// Full runs in which no universe test failed, and no test that
    /// [`super::analyse_among`]'s `eligible` left out failed either.
    pub fn green(&self) -> u64 {
        self.green
    }
}
impl Unreached {
    pub fn new(measure: Measure, wanted: f64, highest: f64) -> Self {
        Self {
            measure,
            wanted,
            highest,
        }
    }
    pub fn measure(&self) -> Measure {
        self.measure
    }
    /// The threshold, a percentage.
    pub fn wanted(&self) -> f64 {
        self.wanted
    }
    /// The highest the cover reached, a percentage.
    pub fn highest(&self) -> f64 {
        self.highest
    }
}
impl Split {
    pub fn new(rich: usize, unreached: Vec<Unreached>) -> Self {
        Self { rich, unreached }
    }
    /// The length of the rich prefix of the cover.
    pub fn rich(&self) -> usize {
        self.rich
    }
    /// The thresholds that were never reached, coverage first.
    pub fn unreached(&self) -> &[Unreached] {
        &self.unreached
    }
}
impl<'a> Line<'a> {
    pub fn new(
        class: Class,
        coverage: f64,
        confidence: f64,
        suite: &'a Tainted,
        name: &'a Tainted,
    ) -> Self {
        Self {
            class,
            coverage,
            confidence,
            suite,
            name,
        }
    }
    pub fn class(&self) -> Class {
        self.class
    }
    /// Percentage of the matrix rows caught once this test and every line
    /// before it pass.
    pub fn coverage(&self) -> f64 {
        self.coverage
    }
    /// [`super::Estimator::p_green`] once this test and every line before it pass,
    /// as a percentage.
    pub fn confidence(&self) -> f64 {
        self.confidence
    }
    /// Borrow the suite for the original text lifetime, independent of this line.
    pub fn suite(&self) -> &'a Tainted {
        self.suite
    }
    /// Borrow the name for the original text lifetime, independent of this line.
    pub fn name(&self) -> &'a Tainted {
        self.name
    }
}

/// Completed output lines in display order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lines<'a> {
    lines: Vec<Line<'a>>,
}
impl<'a> Lines<'a> {
    pub fn new(lines: Vec<Line<'a>>) -> Self {
        Self { lines }
    }
    pub fn as_slice(&self) -> &[Line<'a>] {
        &self.lines
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Line<'a>> {
        self.lines.iter()
    }
    pub fn len(&self) -> usize {
        self.lines.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
    /// Consume lines in display order without exposing their backing storage.
    pub fn into_lines(self) -> impl ExactSizeIterator<Item = Line<'a>> + DoubleEndedIterator {
        self.lines.into_iter()
    }
}

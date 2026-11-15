//! greenshift: test selection from test history.
//!
//! This crate is the core: [`junit`] reads JUnit XML, [`store`] keeps the
//! per-repo SQLite history, [`select`] chooses tests from that history,
//! [`nextest`] writes that choice as a cargo-nextest filterset, and [`cli`]
//! parses the command line and dispatches. [`tainted`] holds text from
//! outside the program so that it prints escaped.

pub mod cli;
pub mod junit;
pub mod nextest;
pub mod select;
pub mod store;
pub mod tainted;

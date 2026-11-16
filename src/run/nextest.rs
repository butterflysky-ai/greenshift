//! The cargo-nextest backend: `greenshift run nextest`.
//!
//! It lists with `cargo nextest list --message-format json` and runs each
//! group with `cargo nextest run -E <filterset>`, the filterset naming
//! the group's tests one by one ([`crate::nextest::any_of`]). A
//! filterset longer than the limit on one argument is split into several
//! runs ([`batches`]).
//!
//! JUnit goes to a file of each run's own through a tool config file
//! (`--tool-config-file greenshift:<file>`) that sets the profile's
//! `junit.path` to an absolute path. nextest ranks a tool config file
//! below the repository's `.config/nextest.toml`, so the profile used is
//! one the repository does not configure: `greenshift`, which, like any
//! profile, inherits the repository's `default` profile. A `--profile`
//! among the passed-on arguments, or else `NEXTEST_PROFILE`, is used
//! instead.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context;

use super::{Backend, Invocation, Listed};
use crate::plan::InventoryTest;
use crate::select::Class;
use crate::tainted::Tainted;

/// The profile runs use when no `--profile` is passed on.
pub const PROFILE: &str = "greenshift";

/// The default limit on one filterset's length, in bytes: under Linux's
/// 131,072-byte limit on one argument (MAX_ARG_STRLEN, its NUL
/// included), with room to spare.
pub const MAX_FILTERSET_BYTES: usize = 100_000;

/// The program run: cargo, which runs cargo-nextest.
const CARGO: &str = "cargo";

/// The cargo-nextest backend, set up from the arguments to pass on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nextest {
    /// The profile to list and run with.
    pub profile: String,
    /// The passed-on arguments `cargo nextest list` takes.
    pub list_args: Vec<OsString>,
    /// The passed-on arguments `cargo nextest run` takes.
    pub run_args: Vec<OsString>,
    /// The longest filterset given in one run, in bytes.
    pub max_filterset_bytes: usize,
}

/// Where a passed-on option goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Goes {
    /// `cargo nextest run` only: list refuses it.
    RunOnly,
    /// `cargo nextest list` only: it chooses tests, and the listing's
    /// choice reaches run as the tests the filterset names. Given to run
    /// as well, a filterset would add to that choice (nextest runs the
    /// tests that match any `-E`) and a partition would split it again.
    ListOnly,
    /// Neither: greenshift passes it itself.
    Profile,
}

/// The passed-on options that do not go to both list and run: each
/// spelling, whether it takes a value, and where it goes. Anything else,
/// test name filters and everything after a `--` included, goes to both.
const OPTIONS: &[(&[&str], bool, Goes)] = &[
    (&["-P", "--profile"], true, Goes::Profile),
    (
        &["-E", "--filterset", "--filter-expr"],
        true,
        Goes::ListOnly,
    ),
    (&["--partition"], true, Goes::ListOnly),
    (
        &["-T", "--message-format", "--message-format-version"],
        true,
        Goes::RunOnly,
    ),
    (&["-j", "--test-threads", "--jobs"], true, Goes::RunOnly),
    (&["-R", "--rerun"], true, Goes::RunOnly),
    (
        &[
            "--debugger",
            "--failure-output",
            "--final-status-level",
            "--flaky-result",
            "--max-fail",
            "--max-progress-running",
            "--no-tests",
            "--retries",
            "--status-level",
            "--stress-count",
            "--stress-duration",
            "--success-output",
            "--tracer",
        ],
        true,
        Goes::RunOnly,
    ),
    (
        &[
            "--fail-fast",
            "--ff",
            "--no-fail-fast",
            "--nff",
            "--hide-progress-bar",
            "--no-capture",
            "--nocapture",
            "--no-input-handler",
            "--no-output-indent",
            "--no-run",
        ],
        false,
        Goes::RunOnly,
    ),
];

/// The entry of [`OPTIONS`] for `arg`, and whether `arg` holds its value
/// too (`--name=value`, or `-Xvalue` for a short option).
fn option(arg: &str) -> Option<((bool, Goes), bool)> {
    for &(names, takes_value, goes) in OPTIONS {
        for name in names {
            let Some(rest) = arg.strip_prefix(name) else {
                continue;
            };
            let short = !name.starts_with("--");
            if rest.is_empty() {
                return Some(((takes_value, goes), false));
            }
            if takes_value && (short || rest.starts_with('=')) {
                return Some(((takes_value, goes), true));
            }
        }
    }
    None
}

impl Nextest {
    /// The backend for the arguments `args` that follow `--` on
    /// greenshift's command line, split by where each goes (see
    /// [`OPTIONS`]). `env_profile` is `NEXTEST_PROFILE`, which nextest
    /// reads for `--profile` when none is given: greenshift always gives
    /// one, so it reads it in nextest's place, empty being unset.
    pub fn new(
        args: &[OsString],
        env_profile: Option<&OsStr>,
        max_filterset_bytes: usize,
    ) -> anyhow::Result<Nextest> {
        let mut profile = None;
        let mut list_args = Vec::new();
        let mut run_args = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                list_args.push(arg.clone());
                run_args.push(arg.clone());
                list_args.extend(args.clone().cloned());
                run_args.extend(args.cloned());
                break;
            }
            let Some(((takes_value, goes), inline)) = arg.to_str().and_then(option) else {
                list_args.push(arg.clone());
                run_args.push(arg.clone());
                continue;
            };
            let mut taken = vec![arg.clone()];
            if takes_value && !inline {
                taken.extend(args.next().cloned());
            }
            match goes {
                Goes::RunOnly => run_args.extend(taken),
                Goes::ListOnly => list_args.extend(taken),
                Goes::Profile => profile = Some(profile_name(&taken)?),
            }
        }
        let profile = match (profile, env_profile) {
            (Some(profile), _) => profile,
            (None, Some(env)) if !env.is_empty() => env
                .to_str()
                .context("NEXTEST_PROFILE needs a profile name in UTF-8")?
                .to_owned(),
            (None, _) => PROFILE.to_owned(),
        };
        Ok(Nextest {
            profile,
            list_args,
            run_args,
            max_filterset_bytes,
        })
    }

    /// `cargo nextest list`'s arguments, its tool config file at `config`.
    pub fn list_command(&self, config: &Path) -> anyhow::Result<Vec<OsString>> {
        let mut args = self.common("list", config)?;
        args.extend(["--message-format", "json"].map(OsString::from));
        args.extend(self.list_args.iter().cloned());
        Ok(args)
    }

    /// `cargo nextest run`'s arguments for the filterset `filterset`, its
    /// tool config file at `config`.
    pub fn run_command(&self, config: &Path, filterset: &str) -> anyhow::Result<Vec<OsString>> {
        let mut args = self.common("run", config)?;
        args.extend(["-E", filterset].map(OsString::from));
        args.extend(self.run_args.iter().cloned());
        Ok(args)
    }

    /// `nextest <command>` with the tool config file and the profile.
    fn common(&self, command: &str, config: &Path) -> anyhow::Result<Vec<OsString>> {
        if !config.is_absolute() {
            anyhow::bail!(
                "nextest needs an absolute tool config path, not {}",
                Tainted::from(config)
            );
        }
        let mut tool = OsString::from("greenshift:");
        tool.push(config);
        Ok(vec![
            "nextest".into(),
            command.into(),
            "--tool-config-file".into(),
            tool,
            "--profile".into(),
            self.profile.clone().into(),
        ])
    }

    /// The tool config file that sends the profile's JUnit to `junit`.
    pub fn tool_config(&self, junit: &Path) -> anyhow::Result<String> {
        let junit = junit
            .to_str()
            .with_context(|| format!("JUnit path {} is not UTF-8", Tainted::from(junit)))?;
        Ok(format!(
            "[profile.{}.junit]\npath = {}\n",
            toml_string(&self.profile),
            toml_string(junit)
        ))
    }
}

/// The profile named by a passed-on `--profile` option, `taken` being the
/// option and its value as given.
fn profile_name(taken: &[OsString]) -> anyhow::Result<String> {
    let arg = taken[0].to_str().unwrap_or_default();
    let name = match taken.get(1) {
        Some(value) => value.to_str(),
        None => arg
            .strip_prefix("--profile=")
            // `-Pci` or, as clap takes it too, `-P=ci`.
            .or_else(|| {
                arg.strip_prefix("-P")
                    .map(|rest| rest.strip_prefix('=').unwrap_or(rest))
            }),
    };
    match name {
        Some(name) if !name.is_empty() => Ok(name.to_owned()),
        _ => anyhow::bail!("--profile needs a profile name, in UTF-8"),
    }
}

/// `text` as a TOML basic string.
fn toml_string(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `terms` joined by ` | ` into filtersets of at most `max` bytes each,
/// in order: every term is in exactly one, and the filtersets, one after
/// the other, give the terms in their order. A term longer than `max` is
/// a filterset alone. No terms give no filtersets.
pub fn batches(terms: &[String], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for term in terms {
        let fits = current.is_empty() || current.len() + " | ".len() + term.len() <= max;
        if !fits {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str(" | ");
        }
        current.push_str(term);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The filtersets that run exactly `tests`: [`batches`] of their terms,
/// or `none()` alone when there are no tests.
pub fn filtersets(tests: &[&InventoryTest], max: usize) -> Vec<String> {
    let terms: Vec<String> = tests
        .iter()
        .map(|test| crate::nextest::term(test.suite.as_raw(), test.name.as_raw()))
        .collect();
    if terms.is_empty() {
        return vec![crate::nextest::any_of([])];
    }
    batches(&terms, max)
}

impl Backend for Nextest {
    fn name(&self) -> &'static str {
        "nextest"
    }

    fn list(&self, dir: &Path) -> anyhow::Result<Listed> {
        let config = dir.join("list.toml");
        std::fs::write(&config, self.tool_config(&dir.join("list.xml"))?)
            .context("could not write nextest's tool config")?;
        let out = std::process::Command::new(CARGO)
            .args(self.list_command(&config)?)
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .context("could not run cargo nextest list")?;
        if !out.status.success() {
            return Ok(Listed::Failed(super::exit_code(out.status).max(1)));
        }
        let json = String::from_utf8(out.stdout).context("cargo nextest list")?;
        let tests = crate::nextest::selected_tests(&json).context("cargo nextest list")?;
        Ok(Listed::Tests(tests))
    }

    fn invocations(
        &self,
        class: Class,
        tests: &[&InventoryTest],
        dir: &Path,
    ) -> anyhow::Result<Vec<Invocation>> {
        let class = match class {
            Class::Rich => "rich",
            Class::Poor => "poor",
        };
        filtersets(tests, self.max_filterset_bytes)
            .iter()
            .enumerate()
            .map(|(at, filterset)| {
                let stem = format!("{class}-{}", at + 1);
                let config = dir.join(format!("{stem}.toml"));
                let junit: PathBuf = dir.join(format!("{stem}.xml"));
                Ok(Invocation {
                    program: CARGO.into(),
                    args: self.run_command(&config, filterset)?,
                    setup: vec![(config, self.tool_config(&junit)?)],
                    junit,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;

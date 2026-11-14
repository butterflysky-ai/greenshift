//! Command-line interface.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::builder::NonEmptyStringValueParser;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::select::{self, Class, Empirical, Estimator, Line, Smoothed, Thresholds, Wilson};
use crate::store::{self, NewRun, Store};
use crate::{junit, nextest};

/// Pick which tests to run from what your tests have done before.
#[derive(Debug, Parser)]
#[command(name = "greenshift", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Record JUnit XML results in the history store, all as one run.
    Ingest(IngestArgs),
    /// Print which tests to run first (rich) and after (poor), from history.
    Select(SelectArgs),
    /// Print select's choice as a cargo-nextest filterset (`-E`).
    NextestFilter(NextestFilterArgs),
}

#[derive(Debug, Args)]
pub struct IngestArgs {
    /// JUnit XML files to record. All of them make up one run.
    #[arg(required = true, value_name = "JUNIT_XML")]
    pub files: Vec<PathBuf>,

    /// History store to write to (created if missing).
    #[arg(long, value_name = "PATH", default_value = store::DEFAULT_PATH)]
    pub db: PathBuf,

    /// Commit the tests ran against.
    #[arg(long, value_name = "SHA", value_parser = NonEmptyStringValueParser::new())]
    pub commit: Option<String>,

    /// Where the results came from, e.g. the test runner or CI job.
    #[arg(
        long,
        value_name = "NAME",
        default_value = "junit",
        value_parser = NonEmptyStringValueParser::new()
    )]
    pub source: String,
}

#[derive(Debug, Args)]
pub struct SelectArgs {
    /// History store to read (never written).
    #[arg(long, value_name = "PATH", default_value = store::DEFAULT_PATH)]
    pub db: PathBuf,

    /// Use only runs from this source; repeat for several. Default: all.
    #[arg(long, value_name = "NAME", value_parser = NonEmptyStringValueParser::new())]
    pub source: Vec<String>,

    /// Results a test needs to be listed as poor; with fewer it is new and
    /// not listed.
    #[arg(long, value_name = "N", default_value_t = 120)]
    pub min_runs: u64,

    /// End rich at the first test whose coverage reaches this percentage
    /// (0 to 100). Default: 100, the whole cover, unless --confidence is
    /// given.
    #[arg(long, value_name = "N", value_parser = percentage)]
    pub coverage: Option<f64>,

    /// End rich at the first test whose confidence reaches this percentage
    /// (0 to 100). With --coverage too, rich ends where both are reached.
    #[arg(long, value_name = "N", value_parser = percentage)]
    pub confidence: Option<f64>,

    /// How confidence is estimated.
    #[arg(long, value_enum, default_value_t = EstimatorKind::Wilson)]
    pub estimator: EstimatorKind,
}

#[derive(Debug, Args)]
pub struct NextestFilterArgs {
    #[command(flatten)]
    pub select: SelectArgs,

    /// The current tests: `cargo nextest list --message-format json`
    /// output (full or `--list-type binaries-only`), or `-` for stdin.
    /// Tests of binaries not in it are left out of the filterset.
    #[arg(long, value_name = "FILE", required = true)]
    pub inventory: PathBuf,

    /// Match the rich tests: every test but the poor ones, so tests new to
    /// the store run too. The default.
    #[arg(long, conflicts_with = "poor")]
    pub rich: bool,

    /// Match the poor tests.
    #[arg(long)]
    pub poor: bool,
}

impl NextestFilterArgs {
    /// Which tests the filterset is to match.
    pub fn class(&self) -> Class {
        if self.poor {
            Class::Poor
        } else {
            Class::Rich
        }
    }
}

/// The confidence estimators `select` offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EstimatorKind {
    /// The lower bound of the Wilson score interval (z = 1.96) for the
    /// share of green runs.
    Wilson,
    /// green / (green + rows left).
    Empirical,
    /// (green + 1) / (green + rows left + 2).
    Smoothed,
}

impl EstimatorKind {
    fn estimator(self) -> &'static dyn Estimator {
        match self {
            EstimatorKind::Wilson => &Wilson,
            EstimatorKind::Empirical => &Empirical,
            EstimatorKind::Smoothed => &Smoothed,
        }
    }
}

fn percentage(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(n) if (0.0..=100.0).contains(&n) => Ok(n),
        _ => Err("expected a percentage from 0 to 100".into()),
    }
}

/// `select`'s and `nextest-filter`'s exit status when there is no full run to select from.
pub const NO_HISTORY: u8 = 2;

/// Parse `std::env::args` and run the requested command.
pub fn run() -> anyhow::Result<ExitCode> {
    match Cli::parse().command {
        Command::Ingest(args) => ingest(&args).map(|()| ExitCode::SUCCESS),
        Command::Select(args) => select(&args),
        Command::NextestFilter(args) => nextest_filter(&args),
    }
}

/// Parse every file first, so a bad file leaves the store untouched, then
/// record them all in one transaction.
pub fn ingest(args: &IngestArgs) -> anyhow::Result<()> {
    let mut cases = Vec::new();
    for file in &args.files {
        let report = junit::parse_file(file).with_context(|| format!("{}", file.display()))?;
        cases.extend(report.cases);
    }
    let mut store = Store::open(&args.db)?;
    let recorded = store.record_run(
        &NewRun {
            commit: args.commit.as_deref(),
            source: &args.source,
            files: &args.files,
        },
        &cases,
    )?;
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    println!(
        "greenshift: recorded run {} with {} test{} from {} file{} into {}",
        recorded.run_id,
        recorded.results,
        plural(recorded.results),
        args.files.len(),
        plural(args.files.len()),
        args.db.display()
    );
    Ok(())
}

/// Print the selection. Reads the store only; with no full run from the
/// chosen sources (or no store at all), says so and exits [`NO_HISTORY`].
pub fn select(args: &SelectArgs) -> anyhow::Result<ExitCode> {
    with_lines(args, |lines| {
        let mut out = std::io::stdout().lock();
        lines
            .iter()
            .try_for_each(|line| writeln!(out, "{line}"))
            .and_then(|()| out.flush())
    })
}

/// Print the selection as one nextest filterset, from the same lines as
/// [`select`] and with the same exits. The inventory is read first, so a
/// bad one fails before the store is read.
pub fn nextest_filter(args: &NextestFilterArgs) -> anyhow::Result<ExitCode> {
    let shown = args.inventory.display();
    let json = if args.inventory.as_os_str() == "-" {
        std::io::read_to_string(std::io::stdin().lock()).context("inventory on stdin")?
    } else {
        std::fs::read_to_string(&args.inventory).with_context(|| format!("inventory {shown}"))?
    };
    let binaries = nextest::binary_ids(&json).with_context(|| format!("inventory {shown}"))?;
    with_lines(&args.select, |lines| {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{}", nextest::filter(lines, args.class(), &binaries))
            .and_then(|()| out.flush())
    })
}

/// Compute `select`'s lines from the store and hand them to `write`, then
/// name any threshold not reached on stderr. With no full run from the
/// chosen sources (or no store at all), says so and exits [`NO_HISTORY`]
/// without calling `write`.
fn with_lines(
    args: &SelectArgs,
    write: impl FnOnce(&[Line<'_>]) -> std::io::Result<()>,
) -> anyhow::Result<ExitCode> {
    let history = match Store::open_read_only(&args.db)? {
        Some(store) => store.history(&args.source)?,
        None => Default::default(),
    };
    let Some(analysis) = select::analyse(&history) else {
        eprintln!("no history: run the full suite");
        return Ok(ExitCode::from(NO_HISTORY));
    };
    let estimator = args.estimator.estimator();
    let decided = select::split(
        &select::cover_coverages(&analysis),
        &select::cover_confidences(&analysis, estimator),
        Thresholds {
            coverage: args.coverage,
            confidence: args.confidence,
        },
    );
    let lines = select::lines(&analysis, estimator, decided.rich, args.min_runs);
    match write(&lines) {
        // A reader that stops early (`| head`) has all the lines it wants.
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        written => written?,
    }
    for missed in decided.unreached {
        eprintln!("greenshift: {missed}");
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn ingest_defaults() {
        let cli = Cli::try_parse_from(["greenshift", "ingest", "a.xml", "b.xml"]).unwrap();
        let Command::Ingest(args) = cli.command else {
            panic!("not ingest");
        };
        assert_eq!(args.files, [PathBuf::from("a.xml"), PathBuf::from("b.xml")]);
        assert_eq!(args.db, PathBuf::from(".greenshift/history.sqlite"));
        assert_eq!(args.commit, None);
        assert_eq!(args.source, "junit");
    }

    #[test]
    fn ingest_options() {
        let cli = Cli::try_parse_from([
            "greenshift",
            "ingest",
            "--db",
            "h.sqlite",
            "--commit",
            "abc123",
            "--source",
            "nextest",
            "a.xml",
        ])
        .unwrap();
        let Command::Ingest(args) = cli.command else {
            panic!("not ingest");
        };
        assert_eq!(args.db, PathBuf::from("h.sqlite"));
        assert_eq!(args.commit.as_deref(), Some("abc123"));
        assert_eq!(args.source, "nextest");
    }

    #[test]
    fn ingest_needs_a_file_and_non_empty_options() {
        assert!(Cli::try_parse_from(["greenshift", "ingest"]).is_err());
        assert!(Cli::try_parse_from(["greenshift", "ingest", "--commit", "", "a.xml"]).is_err());
        assert!(Cli::try_parse_from(["greenshift", "ingest", "--source", "", "a.xml"]).is_err());
    }

    fn select_args(args: &[&str]) -> Result<SelectArgs, clap::Error> {
        let cli = Cli::try_parse_from(["greenshift", "select"].iter().chain(args))?;
        let Command::Select(args) = cli.command else {
            panic!("not select");
        };
        Ok(args)
    }

    #[test]
    fn select_defaults() {
        let args = select_args(&[]).unwrap();
        assert_eq!(args.db, PathBuf::from(".greenshift/history.sqlite"));
        assert_eq!(args.source, Vec::<String>::new());
        assert_eq!(args.min_runs, 120);
        assert_eq!(args.coverage, None);
        assert_eq!(args.confidence, None);
        assert_eq!(args.estimator, EstimatorKind::Wilson);
    }

    #[test]
    fn select_options() {
        let args = select_args(&[
            "--db",
            "h.sqlite",
            "--source",
            "ci",
            "--source",
            "nightly",
            "--min-runs",
            "7",
            "--coverage",
            "90",
            "--confidence",
            "99.5",
            "--estimator",
            "smoothed",
        ])
        .unwrap();
        assert_eq!(args.db, PathBuf::from("h.sqlite"));
        assert_eq!(args.source, ["ci", "nightly"]);
        assert_eq!(args.min_runs, 7);
        assert_eq!(args.coverage, Some(90.0));
        assert_eq!(args.confidence, Some(99.5));
        assert_eq!(args.estimator, EstimatorKind::Smoothed);
        for (name, kind) in [
            ("wilson", EstimatorKind::Wilson),
            ("empirical", EstimatorKind::Empirical),
        ] {
            assert_eq!(select_args(&["--estimator", name]).unwrap().estimator, kind);
        }
    }

    #[test]
    fn select_thresholds_are_percentages() {
        for flag in ["--coverage", "--confidence"] {
            for ok in ["0", "100", "50.25"] {
                assert!(select_args(&[flag, ok]).is_ok(), "{flag} {ok}");
            }
            for bad in ["-1", "100.1", "NaN", "inf", "high", ""] {
                assert!(select_args(&[flag, bad]).is_err(), "{flag} {bad}");
            }
        }
    }

    fn nextest_filter_args(args: &[&str]) -> Result<NextestFilterArgs, clap::Error> {
        let cli = Cli::try_parse_from(
            ["greenshift", "nextest-filter", "--inventory", "inv.json"]
                .iter()
                .chain(args),
        )?;
        let Command::NextestFilter(args) = cli.command else {
            panic!("not nextest-filter");
        };
        Ok(args)
    }

    #[test]
    fn nextest_filter_takes_select_options_and_a_class() {
        let args = nextest_filter_args(&[]).unwrap();
        assert_eq!(args.class(), Class::Rich);
        assert_eq!(args.inventory, PathBuf::from("inv.json"));
        assert!(Cli::try_parse_from(["greenshift", "nextest-filter"]).is_err());
        assert_eq!(args.select.db, PathBuf::from(".greenshift/history.sqlite"));
        assert_eq!(args.select.min_runs, 120);
        let args = nextest_filter_args(&[
            "--db",
            "h.sqlite",
            "--source",
            "nextest",
            "--min-runs",
            "3",
            "--coverage",
            "90",
            "--confidence",
            "80",
            "--estimator",
            "empirical",
            "--poor",
        ])
        .unwrap();
        assert_eq!(args.class(), Class::Poor);
        assert_eq!(args.select.db, PathBuf::from("h.sqlite"));
        assert_eq!(args.select.source, ["nextest"]);
        assert_eq!(args.select.min_runs, 3);
        assert_eq!(args.select.coverage, Some(90.0));
        assert_eq!(args.select.confidence, Some(80.0));
        assert_eq!(args.select.estimator, EstimatorKind::Empirical);
        assert_eq!(
            nextest_filter_args(&["--rich"]).unwrap().class(),
            Class::Rich
        );
        assert!(nextest_filter_args(&["--rich", "--poor"]).is_err());
        assert!(nextest_filter_args(&["--coverage", "101"]).is_err());
    }

    #[test]
    fn select_refuses_bad_options() {
        assert!(select_args(&["--source", ""]).is_err());
        assert!(select_args(&["--min-runs", "-1"]).is_err());
        assert!(select_args(&["--estimator", "bayes"]).is_err());
    }
}

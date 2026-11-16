//! Command-line interface.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::builder::{NonEmptyStringValueParser, StyledStr};
use clap::error::{ContextKind, ContextValue};
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::select::{self, Class, Empirical, Estimator, Line, Smoothed, Thresholds, Wilson};
use crate::store::{self, CommitId, NewRun, Store};
use crate::tainted::Tainted;
use crate::{junit, nextest, plan, run};

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
    /// Print select's choice for the current tests as JSON rich and poor
    /// groups, for any test runner.
    Plan(PlanArgs),
    /// List the tests, run the rich and then the poor ones with a test
    /// runner, and record the results as one run.
    Run(RunArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(subcommand)]
    pub backend: Backend,
}

/// The test runners `run` drives.
#[derive(Debug, Subcommand)]
pub enum Backend {
    /// Run the tests with cargo-nextest.
    Nextest(NextestRunArgs),
}

/// What `run` takes whatever the test runner.
#[derive(Debug, Args)]
#[command(mut_arg("db", |db| {
    db.help("History store to read and record the run in (created if missing)")
}))]
pub struct RunOptions {
    #[command(flatten)]
    pub select: SelectArgs,

    /// Commit the tests run against, as for `ingest --commit`.
    #[arg(long, value_name = "SHA")]
    pub commit: Option<CommitId>,
}

#[derive(Debug, Args)]
#[command(mut_arg("source", |source| {
    source
        .action(clap::ArgAction::Set)
        .default_value("nextest")
        .help("Read only runs from this source, and record the run as from it")
}))]
pub struct NextestRunArgs {
    #[command(flatten)]
    pub options: RunOptions,

    /// The longest filterset to give one `cargo nextest run`, in bytes;
    /// a longer one is split over several runs.
    #[arg(long, value_name = "BYTES", default_value_t = run::nextest::MAX_FILTERSET_BYTES,
          value_parser = clap::value_parser!(usize))]
    pub max_filterset_bytes: usize,

    /// Arguments for `cargo nextest run` and, where it takes them,
    /// `cargo nextest list`.
    #[arg(last = true, value_name = "NEXTEST_ARGS")]
    pub args: Vec<std::ffi::OsString>,
}

#[derive(Debug, Args)]
pub struct IngestArgs {
    /// JUnit XML files to record. All of them make up one run.
    #[arg(required = true, value_name = "JUNIT_XML")]
    pub files: Vec<PathBuf>,

    /// History store to write to (created if missing).
    #[arg(long, value_name = "PATH", default_value = store::DEFAULT_PATH)]
    pub db: PathBuf,

    /// Commit the tests ran against: 7 to 64 hex digits, as
    /// `git rev-parse HEAD` prints it. Stored in lower case.
    #[arg(long, value_name = "SHA")]
    pub commit: Option<CommitId>,

    /// Where the results came from, e.g. the test runner or CI job.
    #[arg(
        long,
        value_name = "NAME",
        default_value = "junit",
        value_parser = NonEmptyStringValueParser::new()
    )]
    pub source: String,

    /// The files hold only part of the suite (one pass of several, say,
    /// when another wrote no JUnit). Such a run never counts as full, but
    /// its results still count towards each test's results.
    #[arg(long)]
    pub partial: bool,
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

#[derive(Debug, Args)]
#[command(mut_arg("db", |db| {
    db.help("History store to read (created empty if missing, else never written)")
}))]
pub struct PlanArgs {
    #[command(flatten)]
    pub select: SelectArgs,

    /// The current tests, one `{"suite": ..., "name": ...}` JSON object
    /// per line, or `-` for stdin. Each lands in exactly one group.
    #[arg(long, value_name = "FILE", required = true)]
    pub inventory: PathBuf,
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

/// `select`'s exit status when there is no full run to select from.
pub const NO_HISTORY: u8 = 2;

/// Parse a command line, `args[0]` being the program name.
///
/// clap's usage errors quote what was given on the command line as it was
/// given: a rejected value, an unknown argument or subcommand. Those
/// quotes are escaped here, as [`Tainted`] shows text, so a terminal
/// escape sequence or newline in argv prints harmlessly. The rest of the
/// error, styling included, is clap's.
pub fn parse<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(args).map_err(escape_argv)
}

/// The clap error, with the parts it quotes from argv escaped.
fn escape_argv(mut err: clap::Error) -> clap::Error {
    let mut quoted = Vec::new();
    for kind in [
        ContextKind::InvalidArg,
        ContextKind::InvalidValue,
        ContextKind::InvalidSubcommand,
    ] {
        if let Some(ContextValue::String(raw)) = err.get(kind) {
            let shown = Tainted::raw(raw.as_str()).to_string();
            if shown != *raw {
                quoted.push((raw.clone(), shown.clone()));
                err.insert(kind, ContextValue::String(shown));
            }
        }
    }
    if quoted.is_empty() {
        return err;
    }
    // clap's tips ("to pass '--x' as a value, use '-- --x'") quote argv
    // too, inside styled text.
    if let Some(ContextValue::StyledStrs(tips)) = err.get(ContextKind::Suggested) {
        let tips: Option<Vec<StyledStr>> =
            tips.iter().map(|tip| escape_tip(tip, &quoted)).collect();
        match tips {
            Some(tips) => err.insert(ContextKind::Suggested, ContextValue::StyledStrs(tips)),
            None => err.remove(ContextKind::Suggested),
        };
    }
    err
}

/// The tip with each quote from argv replaced by its escaped form,
/// keeping clap's styling, or `None` if a quote's occurrences in the
/// styled text overlap, so that which are argv cannot be told.
fn escape_tip(tip: &StyledStr, quoted: &[(String, String)]) -> Option<StyledStr> {
    let mut text = tip.ansi().to_string();
    for (raw, shown) in quoted {
        let starts: Vec<usize> = text
            .char_indices()
            .map(|(at, _)| at)
            .filter(|&at| text[at..].starts_with(raw.as_str()))
            .collect();
        if starts.windows(2).any(|pair| pair[1] - pair[0] < raw.len()) {
            return None;
        }
        text = text.replace(raw.as_str(), shown);
    }
    Some(StyledStr::from(text))
}

/// Parse `std::env::args_os` and run the requested command.
pub fn run() -> anyhow::Result<ExitCode> {
    let cli = parse(std::env::args_os()).unwrap_or_else(|err| err.exit());
    match cli.command {
        Command::Ingest(args) => ingest(&args).map(|()| ExitCode::SUCCESS),
        Command::Select(args) => select(&args),
        Command::NextestFilter(args) => nextest_filter(&args),
        Command::Plan(args) => plan(&args),
        Command::Run(args) => match args.backend {
            Backend::Nextest(args) => run_nextest(&args),
        },
    }
}

/// Name `err` on stderr as greenshift names every error, whether it ends
/// the run (from `main`) or is planned past (by [`plan()`]).
pub fn show_error(err: &anyhow::Error) {
    eprintln!("greenshift: error: {err:#}");
}

/// Parse every file first, so a bad file leaves the store untouched, then
/// record them all in one transaction.
pub fn ingest(args: &IngestArgs) -> anyhow::Result<()> {
    let mut cases = Vec::new();
    for file in &args.files {
        let report = junit::parse_file(file).with_context(|| Tainted::from(file.as_path()))?;
        cases.extend(report.cases);
    }
    let mut store = Store::open(&args.db)?;
    let recorded = store.record_run(
        &NewRun {
            commit: args.commit.as_ref(),
            source: &args.source,
            files: &args.files,
            partial: args.partial,
        },
        &cases,
    )?;
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    println!(
        "greenshift: recorded {}run {} with {} test{} from {} file{} into {}",
        if args.partial { "partial " } else { "" },
        recorded.run_id,
        recorded.results,
        plural(recorded.results),
        args.files.len(),
        plural(args.files.len()),
        Tainted::from(args.db.as_path())
    );
    Ok(())
}

/// Print the selection. Reads the store only; with no full run from the
/// chosen sources (or no store at all), says so and exits [`NO_HISTORY`].
pub fn select(args: &SelectArgs) -> anyhow::Result<ExitCode> {
    let every_test = |_: &Tainted, _: &Tainted| true;
    with_lines(args, every_test, |lines| {
        let Some(lines) = lines else {
            return Ok(ExitCode::from(NO_HISTORY));
        };
        let mut out = std::io::stdout().lock();
        lines
            .iter()
            .try_for_each(|line| writeln!(out, "{line}"))
            .and_then(|()| out.flush())
            .map(|()| ExitCode::SUCCESS)
    })
}

/// Print the selection as one nextest filterset, from lines made as
/// [`select()`] makes them but chosen from the inventory's tests only
/// ([`nextest::Listing::has`]), so rich is never a test the inventory
/// shows is gone.
/// With no history there are no lines, so every test is rich: it prints
/// `all()` or `none()` and exits 0. The inventory is read first, so a bad
/// one fails before the store is read.
pub fn nextest_filter(args: &NextestFilterArgs) -> anyhow::Result<ExitCode> {
    let listing = read_inventory(&args.inventory, nextest::listing)?;
    let eligible = |suite: &Tainted, name: &Tainted| listing.has(suite.as_raw(), name.as_raw());
    with_lines(&args.select, eligible, |lines| {
        let mut out = std::io::stdout().lock();
        let lines = lines.unwrap_or_default();
        let filter = nextest::filter(lines, args.class(), &listing.binaries);
        writeln!(out, "{filter}")
            .and_then(|()| out.flush())
            .map(|()| ExitCode::SUCCESS)
    })
}

/// Print the plan: the inventory split into rich and poor, from lines made
/// as [`select()`] makes them but chosen from the inventory's tests only,
/// by suite and name, as one line of JSON. A missing store is created
/// empty, through [`Store::open`]; an existing one is only read. With no
/// history there are no lines, so every test is rich. A store that cannot
/// be created or read is no history too: the plan's `history` is `error`,
/// stderr names the error, and it still exits 0, so a broken store never
/// stops the tests from running. The inventory is read first, so a bad
/// one fails, with nothing printed, before the store is touched.
pub fn plan(args: &PlanArgs) -> anyhow::Result<ExitCode> {
    let tests = read_inventory(&args.inventory, plan::parse_inventory)?;
    let (history, groups) = plan_groups(&args.select, &tests)?;
    let mut out = std::io::stdout().lock();
    written(
        writeln!(out, "{}", groups.to_json(history))
            .and_then(|()| out.flush())
            .map(|()| ExitCode::SUCCESS),
    )
}

/// `tests` split into rich and poor as [`plan()`] splits them, from lines
/// made as [`select()`] makes them but chosen from `tests` alone, by
/// suite and name, and whether the store could be used. A missing store
/// is created empty; an existing one is only read. With no history, or a
/// store that cannot be created or read, every test is rich; stderr says
/// which, naming the error.
pub fn plan_groups<'t>(
    args: &SelectArgs,
    tests: &'t [plan::InventoryTest],
) -> anyhow::Result<(plan::HistoryState, plan::Plan<'t>)> {
    let history = match plan_history(args) {
        Ok(history) => history,
        Err(err) => {
            show_error(&err);
            return Ok((plan::HistoryState::Error, plan::plan(&[], tests)));
        }
    };
    let known: HashSet<(&Tainted, &Tainted)> =
        tests.iter().map(|test| (&test.suite, &test.name)).collect();
    let eligible = |suite: &Tainted, name: &Tainted| known.contains(&(suite, name));
    let mut groups = None;
    with_history(&history, args, eligible, |lines| {
        groups = Some(plan::plan(lines.unwrap_or_default(), tests));
        Ok(ExitCode::SUCCESS)
    })?;
    let groups = groups.expect("with_history hands the lines on");
    Ok((plan::HistoryState::Ok, groups))
}

/// Run the whole cycle with cargo-nextest; see [`run`].
pub fn run_nextest(args: &NextestRunArgs) -> anyhow::Result<ExitCode> {
    let backend = run::nextest::Nextest::new(&args.args, args.max_filterset_bytes)?;
    run::run(&backend, &args.options)
}

/// The history `plan` reads. A missing store is created, and its history
/// read through the same connection: none, unless another process wrote
/// runs to it after the check found it missing.
fn plan_history(args: &SelectArgs) -> anyhow::Result<store::History> {
    // Only a store known to be missing is created; any other trouble
    // finding it is left to the read below to name.
    if matches!(args.db.try_exists(), Ok(false)) {
        return Ok(Store::open(&args.db)?.history(&args.source)?);
    }
    read_history(args)
}

/// Read the inventory at `path` (`-` for stdin) and `parse` it, with any
/// error naming the inventory.
fn read_inventory<T, E>(
    path: &std::path::Path,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> anyhow::Result<T>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let context = || InventoryContext(Tainted::from(path));
    let text = if path.as_os_str() == "-" {
        std::io::read_to_string(std::io::stdin().lock()).context("inventory on stdin")?
    } else {
        std::fs::read_to_string(path).with_context(context)?
    };
    parse(&text).with_context(context)
}

/// The error context naming the inventory file: the path is kept raw and
/// escaped only when the error is shown.
#[derive(Debug)]
struct InventoryContext(Tainted);

impl std::fmt::Display for InventoryContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inventory {}", self.0)
    }
}

/// Compute `select`'s lines from the store and hand them to `write`, which
/// decides the exit status, then name any threshold not reached on stderr.
/// With no full run from the chosen sources (or no store at all), says so
/// on stderr and hands `write` `None`. The lines are chosen from the
/// tests `eligible` keeps, by suite and name; see [`select::analyse_among`].
fn with_lines(
    args: &SelectArgs,
    eligible: impl Fn(&Tainted, &Tainted) -> bool,
    write: impl FnOnce(Option<&[Line<'_>]>) -> std::io::Result<ExitCode>,
) -> anyhow::Result<ExitCode> {
    with_history(&read_history(args)?, args, eligible, write)
}

/// The history of the chosen sources in the store, read only: none if
/// there is no store.
fn read_history(args: &SelectArgs) -> anyhow::Result<store::History> {
    Ok(match Store::open_read_only(&args.db)? {
        Some(store) => store.history(&args.source)?,
        None => Default::default(),
    })
}

/// [`with_lines`] from `history`, already read.
fn with_history(
    history: &store::History,
    args: &SelectArgs,
    eligible: impl Fn(&Tainted, &Tainted) -> bool,
    write: impl FnOnce(Option<&[Line<'_>]>) -> std::io::Result<ExitCode>,
) -> anyhow::Result<ExitCode> {
    let Some(analysis) = select::analyse_among(history, eligible) else {
        eprintln!("no history: run the full suite");
        return written(write(None));
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
    let code = written(write(Some(&lines)))?;
    for missed in decided.unreached {
        eprintln!("greenshift: {missed}");
    }
    Ok(code)
}

/// The exit status from writing output. A reader that stops early
/// (`| head`) has all the lines it wants, so that is a success.
fn written(result: std::io::Result<ExitCode>) -> anyhow::Result<ExitCode> {
    match result {
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
        result => Ok(result?),
    }
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
        assert!(!args.partial);
    }

    #[test]
    fn ingest_options() {
        let cli = Cli::try_parse_from([
            "greenshift",
            "ingest",
            "--db",
            "h.sqlite",
            "--commit",
            "abc1234",
            "--source",
            "nextest",
            "--partial",
            "a.xml",
        ])
        .unwrap();
        let Command::Ingest(args) = cli.command else {
            panic!("not ingest");
        };
        assert_eq!(args.db, PathBuf::from("h.sqlite"));
        assert_eq!(args.commit.as_ref().map(CommitId::as_str), Some("abc1234"));
        assert_eq!(args.source, "nextest");
        assert!(args.partial);
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
    fn plan_takes_select_options_and_an_inventory() {
        assert!(Cli::try_parse_from(["greenshift", "plan"]).is_err());
        let cli = Cli::try_parse_from([
            "greenshift",
            "plan",
            "--inventory",
            "-",
            "--db",
            "h.sqlite",
            "--source",
            "lein",
            "--min-runs",
            "3",
            "--coverage",
            "90",
            "--estimator",
            "smoothed",
        ])
        .unwrap();
        let Command::Plan(args) = cli.command else {
            panic!("not plan");
        };
        assert_eq!(args.inventory, PathBuf::from("-"));
        assert_eq!(args.select.db, PathBuf::from("h.sqlite"));
        assert_eq!(args.select.source, ["lein"]);
        assert_eq!(args.select.min_runs, 3);
        assert_eq!(args.select.coverage, Some(90.0));
        assert_eq!(args.select.estimator, EstimatorKind::Smoothed);
        assert!(Cli::try_parse_from(["greenshift", "plan", "--inventory", "-", "--poor"]).is_err());
    }

    #[test]
    fn a_tip_has_each_quote_from_argv_escaped() {
        let quoted = [("\u{1b}x".to_owned(), r"\u{1b}x".to_owned())];
        let tip = StyledStr::from("pass '\u{1b}x', use '-- \u{1b}x'".to_owned());
        let tip = escape_tip(&tip, &quoted).unwrap();
        assert_eq!(tip.to_string(), r"pass '\u{1b}x', use '-- \u{1b}x'");
    }

    #[test]
    fn a_tip_whose_quotes_overlap_is_dropped() {
        // "aXa" occurs at 0 and 2: which one is argv cannot be told.
        let quoted = [("a\u{1b}a".to_owned(), r"a\u{1b}a".to_owned())];
        let tip = StyledStr::from("a\u{1b}a\u{1b}a".to_owned());
        assert!(escape_tip(&tip, &quoted).is_none());
    }

    #[test]
    fn parse_escapes_a_rejected_value_once() {
        let err = parse(["greenshift", "ingest", "--commit", "\u{1b}[31m", "a.xml"]).unwrap_err();
        assert_eq!(
            err.get(ContextKind::InvalidValue),
            Some(&ContextValue::String(r"\u{1b}[31m".to_owned()))
        );
    }

    #[test]
    fn parse_drops_a_tip_whose_argv_quotes_overlap() {
        // clap's tip reads "to pass '{arg}' as a value, use '-- {arg}'",
        // with styling after each quote. This argument ends with the text
        // between its two quotes, so its occurrences in the tip overlap.
        let arg = "--\u{1b}[0m' as a value, use '\u{1b}[32m-- --";
        let err = parse(["greenshift", "ingest", "a.xml", arg]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
        assert!(err.get(ContextKind::Suggested).is_none(), "{err:?}");
        let shown = err.render().ansi().to_string();
        assert!(!shown.contains(arg), "{shown:?}");
    }

    #[test]
    fn an_inventory_error_keeps_the_path_raw_and_shows_it_escaped_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("\u{1b}[31mevil.json");
        let mut args = nextest_filter_args(&[]).unwrap();
        args.inventory = path.clone();
        let err = nextest_filter(&args).unwrap_err();
        let context = err
            .downcast_ref::<InventoryContext>()
            .unwrap_or_else(|| panic!("not an inventory error: {err:?}"));
        assert_eq!(context.0.as_raw(), path.to_string_lossy());
        let shown = format!("{err:#}");
        assert!(shown.contains(r"\u{1b}[31mevil.json"), "{shown:?}");
        assert!(!shown.contains(r"\\u{1b}"), "escaped twice: {shown:?}");
        assert!(!shown.contains('\u{1b}'), "{shown:?}");
    }

    #[test]
    fn select_refuses_bad_options() {
        assert!(select_args(&["--source", ""]).is_err());
        assert!(select_args(&["--min-runs", "-1"]).is_err());
        assert!(select_args(&["--estimator", "bayes"]).is_err());
    }
}

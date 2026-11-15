# greenshift

greenshift picks which tests to run from what your tests have done before.

It is early. This release records test history from JUnit XML into a
per-repo SQLite store (`greenshift ingest`), chooses tests from that
history (`greenshift select`) and writes that choice as a cargo-nextest
filterset (`greenshift nextest-filter`) or, for any test runner, as JSON
rich and poor groups of the current tests (`greenshift plan`).

## Ingest

```
greenshift ingest <JUNIT_XML>... [--db PATH] [--commit SHA] [--source NAME]
                  [--partial]
```

Every file given in one invocation is recorded as one run. For example, with
cargo-nextest's JUnit output turned on in `.config/nextest.toml`:

```toml
[profile.default.junit]
path = "junit.xml"
```

```
cargo nextest run
greenshift ingest target/nextest/default/junit.xml \
    --commit "$(git rev-parse HEAD)" --source nextest
```

- `--db` defaults to `.greenshift/history.sqlite`; the directory is created.
  greenshift only writes to its own stores: a SQLite file that already holds
  another program's tables is refused with exit 1 and left untouched.
- `--commit` is optional. It is 7 to 64 hex digits (a full SHA-1 or
  SHA-256 object name, or an abbreviation of one at least 7 long), stored in
  lower case; anything else, such as `HEAD` or a tag, is a usage error
  (exit 2). Runs recorded before this check keep whatever they were given.
  `--source` names the producer or CI job (default `junit`).
- `--partial` records the run as holding only part of the suite: one pass
  of several when another wrote no JUnit, say. `select` never counts a
  partial run as full, whatever its size; its results still count towards
  `--min-runs`.
- greenshift's messages show what was given on the command line (paths,
  and in usage errors a rejected value or an unexpected argument or
  subcommand) with control characters and backslashes escaped as in
  Rust's `{:?}`, so a file name cannot send terminal escape sequences or
  break a line.
- A test is identified by suite and name. The suite is the testcase's
  `classname`, or the enclosing `<testsuite>` name when there is none; it is
  empty when neither is given.
- Outcomes are `passed`, `failed`, `error` and `skipped`; durations are
  stored in whole milliseconds. Failure text and test output are not stored.
- greenshift exits 1 and writes nothing if any file is unreadable or not
  UTF-8, is XML that quick-xml rejects, has no root element or ends before
  it is closed, has a root other than `<testsuites>` or `<testsuite>`, has
  content outside the root element, or has a `<testcase>` without a name or
  with a `time` that is not seconds. Inside a
  `<testcase>`, only which `<failure>`, `<error>` and `<skipped>` children it
  has is read; anything else there, nested testcases included, is ignored.
- An entity or character reference that cannot be resolved, in element text
  or in an attribute greenshift reads, is also an error. Only XML's five
  predefined entities (`&amp;` and so on) are known; entities declared in a
  DTD are not supported. Other attributes are not read. greenshift does not
  otherwise validate XML well-formedness: the input is expected to come from
  a known producer.

Known producers: cargo-nextest and vaelii's test runner. Anything that writes
the common `<testsuites>/<testsuite>/<testcase>` shape should work.

## Select

```
greenshift select [--db PATH] [--source NAME]... [--min-runs N]
                  [--coverage N] [--confidence N]
                  [--estimator wilson|empirical|smoothed]
```

`select` reads the store and writes nothing. It sorts the tests of your
latest full run into *rich* tests, which together have caught every failure
on record, and *poor* tests, which can run after them:

- **Full runs.** Runs are taken oldest first. A run ingested with
  `--partial` is never full. Of the rest, the first is full; after that
  a run is full when it has at least 90% as many results as the largest of
  the three full runs before it, so a short run (say, of only the selected
  tests) is not full either. With `--source`, given once or more, only runs from
  those sources are read; by default all are.
- **Universe.** The tests of the newest full run.
- **Rows.** Every full run in which a universe test failed (`failed` and
  `error` count alike; `skipped` counts as neither a pass nor a failure).
  Full runs with no universe failure are *green*.
- **Rich.** Tests are picked one at a time, highest failure chance per
  second first: f / (f + p) / median, where f is the test's failures in the
  rows not yet covered and p its passes in every full run. Ties go to the
  shorter median, then suite, then name, in byte order. Every row the pick
  fails in is then covered. Picking stops when no row is left, so the rich
  tests together fail in every failing full run; a test that fails in no
  row left is never picked.
- **Poor.** Every other universe test with at least `--min-runs` results
  (default 120) that passed, failed or errored, in any run read; skips do
  not count. The rest are *new* and not listed.
- **Median.** A test's duration is the median of its last 5 non-skipped
  results that have one, in any run read; a test with none gets the median of the other
  tests' medians (1 ms if no test has one). 0 ms counts as 1 ms.

Output is one line per test, rich in pick order, then poor by median, suite
and name, with five tab-separated columns:

```
class	coverage	confidence	suite	name
```

- `class`: `rich` or `poor`.
- `coverage`: the percentage of rows covered once this test and every line
  before it pass, to one decimal. It reads `100.0` only when no row is left,
  and on every poor line.
- `confidence`: an estimate, as a percentage to one decimal, of how likely
  the whole suite is to be green once this test and every line before it
  pass, from the number of green full runs G and the rows left R: of the
  G + R full runs in which those tests passed, G were green. By default
  (`--estimator wilson`) the estimate is the lower end of the Wilson score
  interval for that share, with z = 1.96: a cautious figure that stays low
  while there are few runs to go on, and is 0 when none of them was green.
  `--estimator empirical` is the plain share G / (G + R), taken as 100%
  when both are 0, and `--estimator smoothed` is (G + 1) / (G + R + 2).
- `suite` and `name`: as stored, with control characters, tabs and
  backslashes escaped as in Rust's `{:?}`, so each test is one line.

By default rich is every picked test. `--coverage N` and `--confidence N`
(percentages from 0 to 100) end rich early, at the first picked test whose
coverage or confidence is at least N; the rest of the picked tests are then
treated like any other non-rich test: poor with enough results, otherwise
new. With neither flag, rich behaves as with `--coverage 100`. With both,
rich ends at the first test where both hold, so it is never shorter than
either flag alone would make it. If a threshold is never reached, rich is
all the picked tests and stderr names the threshold and the highest value
reached.

Exit status:

- 0: the selection was printed. That can be nothing at all: with no
  failing full run there is no rich test, and until a test has
  `--min-runs` results it is new, so a young history lists nothing.
- 1: an error, such as a `--db` file that is not a greenshift store.
- 2: no full run from the chosen sources, or no store at all. stderr says
  `no history: run the full suite` and nothing is printed to stdout.

## Nextest filter

```
greenshift nextest-filter --inventory FILE [--rich | --poor]
                          [--db PATH] [--source NAME]...
                          [--min-runs N] [--coverage N] [--confidence N]
                          [--estimator wilson|empirical|smoothed]
```

`nextest-filter` takes `select`'s options, works out the same lines, and
prints one [filterset](https://nexte.st/docs/filtersets/) for
`cargo nextest run -E`. `--inventory` is the current
`cargo nextest list --message-format json` output, full or
`--list-type binaries-only`, as a file or `-` for stdin:

```
cargo nextest list --list-type binaries-only --message-format json \
    > inventory.json
cargo nextest run -E "$(greenshift nextest-filter --source nextest \
    --inventory inventory.json)"
```

- **Terms.** In nextest's JUnit, a testcase's `classname` is the test
  binary's id and its `name` the test's name, so each stored test becomes
  `(binary_id(=<suite>) & test(=<name>))`. A test stored with an empty
  suite becomes `test(=<name>)`, which matches that name in every binary.
- **Inventory.** Tests whose binary id is not in the inventory are left
  out: nextest refuses a whole filterset in which a `binary_id(=...)`
  matches no binary, as one does once a binary is renamed or removed.
  A binaries-only listing ignores `-E`, and a full one keeps binaries it
  filters out (as `skipped`), so neither drops a binary; `-p`,
  `--exclude` and the target-selection flags (`--lib`, `--test` and so
  on) do, so leave them out when making the inventory.
  The option is required, and an inventory that cannot be read or is not
  such a listing is an error (exit 1).
- **`--poor`** prints the terms of the poor tests joined by ` | `, or
  `none()` when there are none.
- **`--rich`**, the default, prints every test but the poor ones:
  `not (<poor terms>)`, or `all()` when there are none. So tests that are
  new, or not in the store at all, run with the rich tests.
- **Escaping.** Within a term, `)`, `,`, `\` and `/` are escaped with a
  backslash, newline, carriage return and tab as `\n`, `\r` and `\t`, and
  other control characters as `\u{HEX}`, so the filterset is one line.
  Everything else, spaces and `(`, `|`, `&` and `::` included, is written
  as is: nextest reads a matcher up to the first unescaped `)` or `,`.

- **Size.** Linux limits one command-line argument to 128 KiB
  (MAX_ARG_STRLEN, 131,072 bytes with its terminating NUL). A term is
  about 92 bytes (greenshift's own 193 tests average 91.5), so
  `-E "$(...)"` fails with `argument list too long` from about
  131,072 / 92 ≈ 1,400 poor tests on. Past that, the filterset has to
  reach nextest another way than as one argument, such as from a file or
  as a profile's `default-filter` in `.config/nextest.toml`.

With no history (where `select` exits 2) there is no poor test, so
`--rich` prints `all()` and `--poor` prints `none()`, and a fresh repo
runs every test; stderr still says `no history: run the full suite`, and
the exit status is 0. Otherwise the exit status, and stderr for a
threshold never reached, are as for `select`.

## Plan

```
greenshift plan --inventory FILE [--db PATH] [--source NAME]...
                [--min-runs N] [--coverage N] [--confidence N]
                [--estimator wilson|empirical|smoothed]
```

`plan` is `nextest-filter` for any test runner. It takes `select`'s
options, works out the same lines, and splits the tests that exist now
into a rich group, to run first, and a poor group, to run after.
`--inventory` lists those tests, one JSON object per line, as a file or
`-` for stdin:

```
{"suite":"vaelii.core-test","name":"parses-a-rule"}
{"suite":"vaelii.core-test","name":"rejects a bent, (odd) name"}
```

It prints one line of JSON:

```
{"rich":[{"suite":"…","name":"…"},…],"poor":[{"suite":"…","name":"…"},…]}
```

- **Tests.** A test is its `suite` and `name`, both required strings
  (the suite may be empty), compared exactly with the store's suite and
  name as ingested. Other members are ignored, blank lines are skipped,
  and a test listed twice is planned once.
- **Groups.** Every inventory test is in exactly one group. Poor is the
  inventory tests on `select`'s poor lines, in `select`'s order. Rich is
  every other inventory test: first `select`'s rich tests in pick order,
  then the rest in inventory order. So tests the store has never seen, and
  those with fewer than `--min-runs` results (`select`'s *new* tests), are
  rich. Stored tests that are not in the inventory are left out.
- **Names.** Suite and name are written as JSON strings of the raw text:
  JSON's own escapes only (`\"`, `\\`, `\t`, `\u001b` and so on), none of
  `select`'s display escaping, so they read back exactly as stored.
  Commas, parentheses, slashes and non-ASCII text are written as they are.

With no history (where `select` exits 2) there is no poor test, so every
inventory test is rich; stderr still says `no history: run the full
suite`. An empty inventory prints `{"rich":[],"poor":[]}`.

Exit status:

- 0: the plan was printed, with or without history. stderr names any
  `--coverage` or `--confidence` threshold never reached, as for
  `select`.
- 1: an error, and nothing is printed to stdout: an inventory that
  cannot be read, or a line of it that is not a JSON object with string
  `suite` and `name` (stderr names the line), or a `--db` file that is
  not a greenshift store.
- 2: a usage error, such as no `--inventory`.

## Building

```
cargo build --release
```

The test suite needs [cargo-nextest](https://nexte.st/) installed: some
tests run it on a scratch project. Run it with `cargo nextest run`, and
the doctests with `cargo test --doc`.

SQLite is bundled, so release builds for `x86_64-unknown-linux-musl` are
fully static.

## License

MIT; see [LICENSE](LICENSE).

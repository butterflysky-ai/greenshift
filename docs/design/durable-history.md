<!-- design-meta: status=implemented-local-live-forge-verification-pending; last-updated=2026-11-19; phase=implementation -->
# Durable history

`refs/greenshift/history` is the canonical complete database. Each publication
replaces a parentless commit containing exactly `history.sqlite.gz`. Reachable
history is bounded to the current snapshot. Replaced objects remain subject to
the forge's garbage collection and retention; this does not promise bounded
physical server storage or immediate deletion.

## Interfaces

```
greenshift history cache-key --remote URL
greenshift history restore --remote URL [--db PATH] [--cache-dir DIRECTORY]
greenshift history publish --remote URL [--db PATH] [--cache-dir DIRECTORY]
                           [--default-branch NAME]
greenshift run nextest --history-remote URL --history-artifact PATH
                      [--history-cache-dir DIRECTORY] [other run options]
```

`run` is opt-in: existing local runs are unchanged. Restore merges fetched history
with an existing local store, preserving unpublished local runs. It happens before test
listing; unavailable canonical history runs all tests as rich. Recording remains
best-effort and the artifact is written after recording even when tests fail.
Storage errors emit `⚠` and preserve the test result. Explicit history commands
also warn and return zero on storage failure. Invalid arguments/configuration or
publication-policy refusals exit 2; cancellation exits 130. Successful publish
prints the new commit identity even if its optional cache copy cannot be saved;
a failed publish prints no identity.

On Unix, authentication is supplied through `GREENSHIFT_HISTORY_TOKEN` for HTTPS
only; authenticated HTTP and redirects are refused. Unauthenticated HTTP and
local paths/file URLs are also supported; SSH/custom helpers are not. Token
authentication on other platforms reports unsupported without breaking local
ingestion/selection. URL-embedded credentials are refused. Git runs in its own
temporary bare repository, with inherited Git configuration removed, hooks
disabled, no checkout, and bounded operation duration/output. Tokens do not enter
command arguments or diagnostics.

The wire protocol currently supports SHA-1 Git repositories; a SHA-256 remote
history identity is refused explicitly rather than treated as a usable snapshot.
The binary owns Ctrl-C handling only for active history transport: cancellation
terminates its isolated Git process group and returns 130. Outside transport,
Ctrl-C keeps normal termination behavior. Library transports install no signal
handler. Test-runner listing/build/run children do not inherit
`GREENSHIFT_HISTORY_TOKEN`.

## Published data and audience

Snapshots preserve complete recorded history, including `run_files.path` exactly
as ingested. Absolute JUnit paths can contain workstation usernames or other
host-specific information; anyone with repository access can read that metadata
from the durable ref. Audit the destination audience before publishing. Ingest
relative JUnit paths when absolute paths should not be shared. The snapshot does
not redact paths or silently change durable run identities.

## Stock cache contract

`cache-key` prints `greenshift-history-COMMIT` or `greenshift-history-empty`.
Only stock `actions/cache/restore` and `actions/cache/save` speak the cache
protocol. The directory contains `COMMIT/history.sqlite.gz` and `COMMIT/commit`.
The cached raw commit is hashed against the current canonical identity, then its
tree is checked against the compressed payload's Git blob identity. Naming a
directory after a commit is not sufficient to authenticate its contents. Invalid
or absent cache data falls back to fetching the canonical ref. No prefix restore
key can make an older cache authoritative.

The reference [GitHub reusable workflow](../../examples/durable-history.github.yml)
is not installed by this change. A caller must choose a published release that
contains these commands and pin its independently verified SHA-256. `init`
generation, per-branch saves/fold-in, multi-invocation keys and cache-only mode are
separate work.

Reusable workflows cannot elevate their caller's permissions. A caller must
grant `contents:write` to the calling job; the called **test** job explicitly
downgrades its token to read-only. For example, after copying the reference:

```yaml
name: Tests
on: [push, pull_request]
jobs:
  test-history:
    permissions:
      contents: write
    uses: ./.github/workflows/history.yml
    with:
      release: v0.1.0 # Replace with the chosen release containing history commands.
      sha256: REPLACE_WITH_INDEPENDENTLY_VERIFIED_RELEASE_SHA256
```

These placeholders must be filled before installation; they do not claim an
existing release already contains the feature. Fork PRs run tests only. Missing
history artifacts warn and skip publication/cache save rather than fail an
otherwise successful workflow.

## Publisher trust boundary

The GitHub test job has `contents:read`, checkout does not persist credentials,
and the publisher receives `contents:write` on a **fresh hosted runner**. It never
checks out or executes test output; only a checksummed release binary parses a
single data artifact. Publication is only for pushes to the configured default
branch, never pull requests (including `pull_request_target`). The CLI repeats
that policy for CI invocations and requires `--default-branch` there.

The parser accepts only known supported schema versions/application id, disallows extra
views/triggers/tables/indexes, verifies integrity and foreign keys, checks types,
IDs/outcomes/durations/ref exclusivity, and caps bytes/rows/field lengths and
expanded in-memory data. Input connections are read-only and defensive with
untrusted schemas disabled. Export constructs a new canonical database using
bound values; no input SQL is executed as SQL. Known older schemas are validated
against their own exact trusted migration-prefix schema before data is projected
into a fresh current database. Newer unknown versions are refused without
altering the source or canonical ref. Output replacement is atomic.

Commit strings from existing history remain bounded historical data, including
arbitrary strings written before commit validation existed. New ingest input
still uses `CommitId` validation. Historical commit strings are copied using
bound SQL values, never used as Git command arguments or shell input.

SQLite migration 4 gives each run a durable 128-bit identity. New runs use SQLite
random bytes. Legacy identities hash typed run metadata, original local id,
ordered files and results, so separately migrated copies of one legacy DB agree.
Local id preserves repeated identical legacy invocations. Already-diverged legacy
stores cannot recover whether identical historical records were independently
recorded or shared; that information was never recorded. Same identity with
conflicting contents is rejected rather than silently replacing history. Merged
runs are numbered in `(ingested_at, history_id)` order: this is deterministic
recorded-time ordering, not proof of actual execution chronology. Timestamp ties
do not merge independent run identities.

Each publish fetches and validates current history, merges by durable identity,
and uses an exact `--force-with-lease` compare-and-swap. Rejection refetches,
remerges and retries once. A second rejection warns. Workflow concurrency reduces
collisions but is not the correctness lock, and pending workflow cancellation is
not a promise that every pushed commit receives a run. Newly produced results
that lose both attempts are best-effort: no later publisher automatically reads
the failed run's artifact. It can be replayed manually only while retained (one
day in the reference workflow). Durable pending delivery/replay is separate work;
already-published canonical history remains protected by CAS.

## Forgejo capability gate

Stock Forgejo runner v13.2.2 exports its automatic repository token to every
step as both `GITHUB_TOKEN` and `FORGEJO_TOKEN`. Stock Forgejo v16.0.3/v17.0.2
grants that token write access outside fork pull requests and ignores workflow
`permissions`. `persist-credentials:false` or only referencing the token in a
publisher step does **not** make its test job read-only. A separate publisher
credential does not revoke the automatic one. This is a platform baseline, not
an additional privilege granted by greenshift, but it does not satisfy this
design's read-only-test-job guarantee.

Consequently no ordinary push-test Forgejo workflow is offered as isolated.
Forge-neutral git transport can be used by authenticated trusted code only with
this limitation understood. Owner reconciliation of the token boundary is
required before claiming Forgejo acceptance. No server/runner configuration or
credentials are changed here. Artifact/cache action compatibility on Forgejo must
be independently tested, not inferred from GitHub's API or action version.

Primary sources:
[runner token export](https://code.forgejo.org/forgejo/runner/src/tag/v13.2.2/act/runner/run_context.go),
[server token permission](https://codeberg.org/forgejo/forgejo/src/tag/v17.0.2/routers/api/v1/permissions/repo_access.go),
[Forgejo compatibility](https://forgejo.org/docs/latest/user/actions/github-actions/).

## Verification boundaries

Local tests cover complete ref round-trips after total cache deletion, stale
snapshot union/replay, malformed cache fallback, tampered artifact/ref unchanged,
legacy identity, independent identical new runs, data bounds and 50k+ rows.
Live GitHub/Forgejo scratch CI, rulesets, fork restoration, actual cache-service
interruption and cleanup listings are separate acceptance receipts; local tests
do not claim those ran.

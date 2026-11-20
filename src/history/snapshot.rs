//! Bounded, data-only SQLite snapshots. Input SQL is never executed as SQL.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rusqlite::{config::DbConfig, params, Connection, OpenFlags};

use crate::junit::Outcome;
use crate::store::{self, Store, APPLICATION_ID, SCHEMA_VERSION};
use crate::tainted::Tainted;

/// Maximum uncompressed snapshot size.
pub const MAX_DB_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum rows in each snapshot table.
pub const MAX_ROWS: i64 = 2_000_000;
/// Maximum bytes in any text or path field.
pub const MAX_FIELD_BYTES: usize = 65_536;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot SQLite error: {0}")]
    Sqlite(Tainted),
    #[error("invalid history snapshot: {0}")]
    Invalid(&'static str),
    #[error("could not create canonical history snapshot: {0}")]
    Store(#[from] crate::store::StoreError),
}

impl From<rusqlite::Error> for SnapshotError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(Tainted::raw(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResultRow {
    suite: String,
    name: String,
    outcome: Outcome,
    duration: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Run {
    history_id: String,
    commit: Option<String>,
    source: String,
    at: String,
    partial: i64,
    branch: Option<String>,
    tag: Option<String>,
    files: Vec<(i64, Vec<u8>)>,
    results: Vec<ResultRow>,
}

struct Data {
    tests: Vec<(String, String)>,
    runs: Vec<Run>,
}

fn invalid(why: &'static str) -> SnapshotError {
    SnapshotError::Invalid(why)
}

struct Budget(usize);

impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<(), SnapshotError> {
        self.0 = self
            .0
            .checked_sub(bytes)
            .ok_or_else(|| invalid("expanded snapshot exceeds memory budget"))?;
        Ok(())
    }
}

type SchemaObject = (String, String, String, Option<String>);

fn schema(conn: &Connection) -> Result<Vec<SchemaObject>, SnapshotError> {
    let mut stmt =
        conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) fn check_sidecars(path: &Path) -> Result<(), SnapshotError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        match std::fs::symlink_metadata(Path::new(&sidecar)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
            Ok(_) => {
                return Err(invalid(
                    "snapshot must be standalone: SQLite sidecar exists",
                ))
            }
        }
    }
    Ok(())
}

fn check_output(path: &Path) -> Result<(), SnapshotError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.is_file() => (),
        Ok(_) => {
            return Err(invalid(
                "snapshot destination must be a regular non-symlink file",
            ))
        }
    }
    // Never remove sidecars: they may belong to a live database. Replacing only
    // its main inode would leave another reader observing the previous WAL.
    check_sidecars(path)
}

fn open_checked(path: &Path) -> Result<(Connection, i64), SnapshotError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_DB_BYTES {
        return Err(invalid("expected a regular SQLite file at most 256 MiB"));
    }
    check_sidecars(path)?;
    let conn = Connection::open_with_flags(
        store::as_plain_file(path),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    // One read transaction covers schema validation and every subsequent query.
    conn.execute_batch("BEGIN")?;
    let pages: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
    let page_size: i64 = conn.pragma_query_value(None, "page_size", |r| r.get(0))?;
    if pages < 0
        || page_size <= 0
        || pages
            .checked_mul(page_size)
            .is_none_or(|n| n > MAX_DB_BYTES as i64)
    {
        return Err(invalid("logical database size exceeds 256 MiB"));
    }
    let app: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if app != APPLICATION_ID {
        return Err(invalid("application id mismatch"));
    }
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(invalid("unsupported snapshot schema version"));
    }
    let expected = store::canonical_schema(version)?;
    if schema(&conn)? != schema(&expected)? {
        return Err(invalid(
            "schema does not exactly match this greenshift release",
        ));
    }
    for table in ["runs", "tests", "results", "run_files"] {
        let count: i64 =
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        if count > MAX_ROWS {
            return Err(invalid("snapshot row limit exceeded"));
        }
    }
    let check: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(invalid("SQLite integrity check failed"));
    }
    if conn
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(invalid("broken foreign key"));
    }
    Ok((conn, version))
}

fn field(text: &str) -> Result<(), SnapshotError> {
    if text.len() > MAX_FIELD_BYTES {
        Err(invalid("text field exceeds 64 KiB"))
    } else {
        Ok(())
    }
}

fn load(path: &Path) -> Result<Data, SnapshotError> {
    load_with_budget(path, MAX_DB_BYTES as usize)
}

fn load_with_budget(path: &Path, expanded_bytes: usize) -> Result<Data, SnapshotError> {
    let (conn, version) = open_checked(path)?;
    // Account expanded names as well as row storage before copying them. A
    // compact SQLite table can share a long name across millions of results.
    // The row allowance includes vector spare capacity and map-node overhead;
    // this is an application allocation budget, not a process RSS guarantee.
    let mut budget = Budget(expanded_bytes);
    let mut tests = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id,suite,name FROM tests ORDER BY id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let suite: String = row.get(1)?;
        let name: String = row.get(2)?;
        if id <= 0 {
            return Err(invalid("test id must be positive"));
        }
        field(&suite)?;
        field(&name)?;
        budget.charge(2 * (std::mem::size_of::<(i64, String, String)>() + 64))?;
        budget.charge(suite.len())?;
        budget.charge(name.len())?;
        tests.insert(id, (suite, name));
    }
    let mut runs = BTreeMap::new();
    let run_sql = match version {
        1 => "SELECT id,commit_sha,source,ingested_at,0,NULL,NULL,NULL FROM runs ORDER BY id",
        2 => "SELECT id,commit_sha,source,ingested_at,partial,NULL,NULL,NULL FROM runs ORDER BY id",
        3 => "SELECT id,commit_sha,source,ingested_at,partial,branch,tag,NULL FROM runs ORDER BY id",
        _ => "SELECT id,commit_sha,source,ingested_at,partial,branch,tag,history_id FROM runs ORDER BY id",
    };
    let mut stmt = conn.prepare(run_sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let run = Run {
            history_id: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            commit: row.get(1)?,
            source: row.get(2)?,
            at: row.get(3)?,
            partial: row.get(4)?,
            branch: row.get(5)?,
            tag: row.get(6)?,
            files: Vec::new(),
            results: Vec::new(),
        };
        if id <= 0 || ![0, 1].contains(&run.partial) || (run.branch.is_some() && run.tag.is_some())
        {
            return Err(invalid("invalid run id, partial flag, or branch/tag"));
        }
        if version >= 4
            && (run.history_id.len() != 32
                || !run
                    .history_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        {
            return Err(invalid("run history id must be 32 lowercase hex digits"));
        }
        for text in [
            run.commit.as_deref(),
            Some(run.source.as_str()),
            Some(run.at.as_str()),
            run.branch.as_deref(),
            run.tag.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            field(text)?;
            budget.charge(text.len())?;
        }
        budget.charge(run.history_id.len())?;
        budget.charge(2 * (std::mem::size_of::<Run>() + 64))?;
        runs.insert(id, run);
    }
    let mut stmt =
        conn.prepare("SELECT run_id,position,path FROM run_files ORDER BY run_id,position")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let position: i64 = row.get(1)?;
        let path: Vec<u8> = row.get(2)?;
        if position < 0 || path.len() > MAX_FIELD_BYTES {
            return Err(invalid("invalid run file position or path length"));
        }
        budget.charge(path.len())?;
        budget.charge(2 * std::mem::size_of::<(i64, Vec<u8>)>())?;
        runs.get_mut(&id)
            .ok_or_else(|| invalid("unknown run for file"))?
            .files
            .push((position, path));
    }
    let mut stmt = conn.prepare(
        "SELECT run_id,test_id,outcome,duration_ms FROM results ORDER BY run_id,test_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let run_id: i64 = row.get(0)?;
        let test_id: i64 = row.get(1)?;
        let outcome: String = row.get(2)?;
        let duration: Option<i64> = row.get(3)?;
        let parsed_outcome = store::stored_outcome(&outcome)
            .ok_or_else(|| invalid("invalid outcome or duration"))?;
        if duration.is_some_and(|d| d < 0) {
            return Err(invalid("invalid outcome or duration"));
        }
        let (suite, name) = tests.get(&test_id).ok_or_else(|| invalid("unknown test"))?;
        budget.charge(suite.len())?;
        budget.charge(name.len())?;
        budget.charge(outcome.len())?;
        budget.charge(2 * std::mem::size_of::<ResultRow>())?;
        runs.get_mut(&run_id)
            .ok_or_else(|| invalid("unknown run for result"))?
            .results
            .push(ResultRow {
                suite: suite.clone(),
                name: name.clone(),
                outcome: parsed_outcome,
                duration,
            });
    }
    if version < 4 {
        budget.charge(
            runs.len()
                .checked_mul(2 * (std::mem::size_of::<(i64, String)>() + 32))
                .ok_or_else(|| invalid("expanded snapshot exceeds memory budget"))?,
        )?;
        for (id, identity) in store::legacy_run_identities(&conn)? {
            runs.get_mut(&id)
                .ok_or_else(|| invalid("legacy identity has no run"))?
                .history_id = identity;
        }
    }
    for run in runs.values_mut() {
        run.results
            .sort_by(|a, b| (&a.suite, &a.name).cmp(&(&b.suite, &b.name)));
    }
    Ok(Data {
        tests: tests.into_values().collect(),
        runs: runs.into_values().collect(),
    })
}

/// Reject any snapshot that is not bounded, canonical greenshift data.
pub fn validate(path: &Path) -> Result<(), SnapshotError> {
    load(path).map(|_| ())
}

fn write(data: &Data, output: &Path) -> Result<(), SnapshotError> {
    check_output(output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    drop(Store::open(temporary.path())?);
    let mut conn = Connection::open(store::as_plain_file(temporary.path()))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let tx = conn.transaction()?;
    let mut test_ids = HashMap::new();
    for (suite, name) in &data.tests {
        if let std::collections::hash_map::Entry::Vacant(entry) =
            test_ids.entry((suite.as_str(), name.as_str()))
        {
            tx.execute(
                "INSERT INTO tests(suite,name) VALUES (?1,?2)",
                params![suite, name],
            )?;
            entry.insert(tx.last_insert_rowid());
        }
    }
    let mut chronological: Vec<_> = data.runs.iter().collect();
    chronological.sort_unstable_by(|a, b| (&a.at, &a.history_id).cmp(&(&b.at, &b.history_id)));
    for run in chronological {
        tx.execute("INSERT INTO runs(commit_sha,source,ingested_at,partial,branch,tag,history_id) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![run.commit,run.source,run.at,run.partial,run.branch,run.tag,run.history_id])?;
        let id = tx.last_insert_rowid();
        for (position, path) in &run.files {
            tx.execute(
                "INSERT INTO run_files(run_id,position,path) VALUES (?1,?2,?3)",
                params![id, position, path],
            )?;
        }
        for result in &run.results {
            let test = test_ids
                .get(&(result.suite.as_str(), result.name.as_str()))
                .ok_or_else(|| invalid("result test missing from snapshot"))?;
            tx.execute(
                "INSERT INTO results(run_id,test_id,outcome,duration_ms) VALUES (?1,?2,?3,?4)",
                params![id, test, result.outcome.as_str(), result.duration],
            )?;
        }
    }
    tx.commit()?;
    drop(conn);
    validate(temporary.path())?;
    temporary.as_file().sync_all()?;
    check_output(output)?;
    temporary
        .persist(output)
        .map_err(|error| SnapshotError::Io(error.error))?;
    Ok(())
}

/// Export one consistent read transaction into a standalone canonical database.
/// Existing output is replaced atomically only after complete validation.
pub fn snapshot(db: &Path, output: &Path) -> Result<(), SnapshotError> {
    write(&load(db)?, output)
}

/// Merge complete histories without treating SQLite's local row ids as identity.
/// Durable run identities make publication idempotent while retaining independently
/// recorded identical runs. Conflicting contents for one identity are rejected.
pub fn merge(current: &Path, incoming: &Path, output: &Path) -> Result<(), SnapshotError> {
    let mut data = load(current)?;
    let other = load(incoming)?;
    let mut additions = Vec::new();
    {
        let known: HashMap<_, _> = data
            .runs
            .iter()
            .map(|run| (run.history_id.as_str(), run))
            .collect();
        for run in other.runs {
            match known.get(run.history_id.as_str()) {
                Some(existing) if **existing != run => {
                    return Err(invalid("one run identity has conflicting contents"))
                }
                Some(_) => (),
                None => additions.push(run),
            }
        }
    }
    data.runs.extend(additions);
    data.tests.extend(other.tests);
    data.tests.sort();
    data.tests.dedup();
    write(&data, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_repeated_shared_name_expansion_is_charged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        fixture(&path, "source");
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE tests SET name=?1", ["x".repeat(1024)])
            .unwrap();
        for id in 2..=10 {
            conn.execute("INSERT INTO runs(id,source,history_id) VALUES (?1,'source',lower(hex(randomblob(16))))", [id]).unwrap();
            conn.execute("INSERT INTO results VALUES (?1,1,'passed',1)", [id])
                .unwrap();
        }
        drop(conn);
        assert!(matches!(
            load_with_budget(&path, 15_000),
            Err(SnapshotError::Invalid(
                "expanded snapshot exceeds memory budget"
            ))
        ));
        assert!(load_with_budget(&path, 100_000).is_ok());
    }

    #[test]
    fn test_budget_arithmetic_exhaustion_rejected() {
        let mut budget = Budget(usize::MAX);
        budget.charge(usize::MAX).unwrap();
        assert!(budget.charge(1).is_err());
    }

    fn fixture(path: &Path, source: &str) {
        drop(Store::open(path).unwrap());
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO runs(source,ingested_at,history_id) VALUES (?1,'2026-01-01T00:00:00.000Z',lower(hex(randomblob(16))))",
            [source],
        )
        .unwrap();
        conn.execute("INSERT INTO tests(suite,name) VALUES ('suite','test')", [])
            .unwrap();
        conn.execute("INSERT INTO results VALUES (1,1,'passed',12)", [])
            .unwrap();
        conn.execute("INSERT INTO run_files VALUES (1,0,x'00ff')", [])
            .unwrap();
    }

    #[test]
    fn test_validate_extra_schema_and_bad_values_rejected() {
        for (change, expected) in [
            ("CREATE TABLE extra(value)", "schema does not exactly match this greenshift release"),
            ("CREATE VIEW extra AS SELECT * FROM tests", "schema does not exactly match this greenshift release"),
            ("CREATE TRIGGER extra AFTER INSERT ON tests BEGIN DELETE FROM results; END", "schema does not exactly match this greenshift release"),
            ("PRAGMA application_id=0", "application id mismatch"),
            ("PRAGMA user_version=999", "unsupported snapshot schema version"),
            ("PRAGMA ignore_check_constraints=ON; UPDATE results SET duration_ms=-1", "invalid outcome or duration"),
            ("PRAGMA foreign_keys=OFF; UPDATE results SET test_id=999", "broken foreign key"),
            ("PRAGMA foreign_keys=OFF; UPDATE runs SET id=-1; UPDATE results SET run_id=-1; UPDATE run_files SET run_id=-1", "invalid run id, partial flag, or branch/tag"),
            ("UPDATE runs SET history_id=NULL", "run history id must be 32 lowercase hex digits"),
            ("UPDATE runs SET history_id='ABCDEF'", "run history id must be 32 lowercase hex digits"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            fixture(&path, "test");
            Connection::open(&path)
                .unwrap()
                .execute_batch(change)
                .unwrap();
            let result=validate(&path);
            assert!(matches!(&result, Err(SnapshotError::Invalid(message)) if *message == expected), "wrong rejection for {change}: {result:?}");
        }
    }

    fn historical_fixture(path: &Path, version: i64) {
        let conn = store::canonical_schema(version).unwrap();
        conn.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
        let output = Connection::open(path).unwrap();
        output.execute("INSERT INTO runs(id,source,ingested_at) VALUES (9,'legacy','2020-01-01T00:00:00.000Z')",[]).unwrap();
        output
            .execute("INSERT INTO tests VALUES (4,'suite','test')", [])
            .unwrap();
        output
            .execute("INSERT INTO results VALUES (9,4,'passed',12)", [])
            .unwrap();
        output
            .execute("INSERT INTO run_files VALUES (9,0,x'00ff')", [])
            .unwrap();
    }

    #[test]
    fn test_snapshot_supported_historical_schemas_upgrade_without_input_mutation() {
        for version in 1..SCHEMA_VERSION {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("input");
            let output = dir.path().join("output");
            historical_fixture(&input, version);
            let before = std::fs::read(&input).unwrap();
            validate(&input).unwrap();
            snapshot(&input, &output).unwrap();
            assert_eq!(std::fs::read(&input).unwrap(), before);
            let upgraded = Store::open_read_only(&output).unwrap().unwrap();
            assert_eq!(upgraded.schema_version().unwrap(), SCHEMA_VERSION);
            assert_eq!(upgraded.history(&[]).unwrap().runs.len(), 1);
        }
    }

    #[test]
    fn test_snapshot_copy_contract_covers_every_canonical_table_and_column() {
        let conn = store::canonical_schema(SCHEMA_VERSION).unwrap();
        let mut tables: Vec<_> = schema(&conn)
            .unwrap()
            .into_iter()
            .filter(|object| object.0 == "table")
            .map(|object| object.1)
            .collect();
        tables.sort();
        assert_eq!(tables, vec!["results", "run_files", "runs", "tests"]);
        for (table, expected) in [
            (
                "runs",
                vec![
                    "id",
                    "commit_sha",
                    "source",
                    "ingested_at",
                    "partial",
                    "branch",
                    "tag",
                    "history_id",
                ],
            ),
            ("tests", vec!["id", "suite", "name"]),
            (
                "results",
                vec!["run_id", "test_id", "outcome", "duration_ms"],
            ),
            ("run_files", vec!["run_id", "position", "path"]),
        ] {
            let columns = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                columns, expected,
                "snapshot copy contract must cover new fields in {table}"
            );
        }
    }

    #[test]
    fn test_snapshot_legacy_trigger_and_future_version_reject_without_writes() {
        for (change, expected) in [
            (
                "CREATE TRIGGER attack AFTER UPDATE ON runs BEGIN DELETE FROM tests; END",
                "schema does not exactly match this greenshift release",
            ),
            (
                "PRAGMA user_version=999",
                "unsupported snapshot schema version",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("input");
            let output = dir.path().join("output");
            historical_fixture(&input, 3);
            Connection::open(&input)
                .unwrap()
                .execute_batch(change)
                .unwrap();
            let before = std::fs::read(&input).unwrap();
            std::fs::write(&output, b"untouched").unwrap();
            assert!(
                matches!(snapshot(&input,&output),Err(SnapshotError::Invalid(message)) if message==expected)
            );
            assert_eq!(std::fs::read(&input).unwrap(), before);
            assert_eq!(std::fs::read(&output).unwrap(), b"untouched");
        }
    }

    #[test]
    fn test_snapshot_legacy_identity_v1_golden_bytes_and_store_migration_agree() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let snapshot = dir.path().join("snapshot");
        historical_fixture(&input, 3);
        super::snapshot(&input, &snapshot).unwrap();
        let identity = |path: &Path| {
            Connection::open(path)
                .unwrap()
                .query_row("SELECT history_id FROM runs", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap()
        };
        // Independent SHA-256 oracle over the fixed v1 typed encoding of the
        // fixture (original id9, NULL commit, legacy source/time/files/results).
        assert_eq!(identity(&snapshot), "29823b29db8f0d9396909bc519396cd2");
        drop(Store::open(&input).unwrap());
        assert_eq!(identity(&input), identity(&snapshot));
    }

    #[test]
    fn test_store_open_invalid_history_identity_refused_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        fixture(&input, "source");
        Connection::open(&input)
            .unwrap()
            .execute("UPDATE runs SET history_id=NULL", [])
            .unwrap();
        let before = std::fs::read(&input).unwrap();
        assert!(matches!(
            Store::open(&input),
            Err(store::StoreError::InvalidHistoryId { .. })
        ));
        assert_eq!(std::fs::read(&input).unwrap(), before);
    }

    #[test]
    fn test_snapshot_legacy_arbitrary_commit_is_bounded_data_and_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        historical_fixture(&input, 3);
        let legacy = "legacy ref; $(not-a-command)\nHEAD";
        Connection::open(&input)
            .unwrap()
            .execute("UPDATE runs SET commit_sha=?1", [legacy])
            .unwrap();
        snapshot(&input, &output).unwrap();
        let conn = Connection::open(&output).unwrap();
        assert_eq!(
            conn.query_row("SELECT commit_sha FROM runs", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            legacy
        );
        assert_eq!(
            conn.query_row("SELECT length(history_id) FROM runs", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            32
        );
        drop(conn);
        Connection::open(&input)
            .unwrap()
            .execute(
                "UPDATE runs SET commit_sha=?1",
                ["x".repeat(MAX_FIELD_BYTES + 1)],
            )
            .unwrap();
        assert!(matches!(
            validate(&input),
            Err(SnapshotError::Invalid("text field exceeds 64 KiB"))
        ));
    }

    #[test]
    fn test_merge_orders_runs_chronologically_then_by_identity_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let early = dir.path().join("early");
        let late = dir.path().join("late");
        let forward = dir.path().join("forward");
        let reverse = dir.path().join("reverse");
        fixture(&early, "early");
        fixture(&late, "late");
        Connection::open(&early).unwrap().execute("UPDATE runs SET ingested_at='2020-01-01T00:00:00.000Z',history_id='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'",[]).unwrap();
        Connection::open(&late).unwrap().execute_batch("UPDATE runs SET ingested_at='2021-01-01T00:00:00.000Z',history_id='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'; INSERT INTO runs(source,ingested_at,history_id) VALUES ('tie','2021-01-01T00:00:00.000Z','00000000000000000000000000000000')").unwrap();
        Connection::open(&late).unwrap().execute_batch("INSERT INTO tests VALUES (2,'suite','new-case'); INSERT INTO results VALUES (1,2,'failed',4)").unwrap();
        merge(&early, &late, &forward).unwrap();
        merge(&late, &early, &reverse).unwrap();
        let order = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            let order = conn
                .prepare("SELECT source FROM runs ORDER BY id")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            order
        };
        assert_eq!(order(&forward), vec!["early", "tie", "late"]);
        assert_eq!(order(&reverse), order(&forward));
        let analyse = |path: &Path| {
            crate::select::analyse(
                &Store::open_read_only(path)
                    .unwrap()
                    .unwrap()
                    .history(&[])
                    .unwrap(),
            )
            .unwrap()
        };
        let forward_selection = analyse(&forward);
        assert!(forward_selection
            .tests
            .iter()
            .any(|test| test.name.as_raw() == "new-case"));
        assert_eq!(analyse(&reverse), forward_selection);
    }

    #[test]
    fn test_snapshot_readback_preserves_metadata_and_binary_paths() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fixture(&input, "producer");
        snapshot(&input, &output).unwrap();
        let db = Connection::open(&output).unwrap();
        assert_eq!(
            db.query_row("SELECT source FROM runs", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "producer"
        );
        assert_eq!(
            db.query_row("SELECT path FROM run_files", [], |r| r.get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![0, 255]
        );
        assert_eq!(
            db.query_row("SELECT duration_ms FROM results", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            12
        );
    }

    #[test]
    fn test_merge_colliding_ids_keeps_disjoint_runs_and_retry_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let output = dir.path().join("output");
        fixture(&first, "first");
        fixture(&second, "second");
        merge(&first, &second, &output).unwrap();
        merge(&output, &second, &first).unwrap();
        let conn = Connection::open(&first).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM results", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM tests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn test_merge_invalid_incoming_leaves_output_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let output = dir.path().join("output");
        fixture(&first, "first");
        std::fs::write(&second, b"not SQLite").unwrap();
        std::fs::write(&output, b"sentinel").unwrap();
        assert!(merge(&first, &second, &output).is_err());
        assert_eq!(std::fs::read(output).unwrap(), b"sentinel");
    }

    #[test]
    fn test_validate_large_text_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        fixture(&path, "test");
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE tests SET name=?1",
                ["x".repeat(MAX_FIELD_BYTES + 1)],
            )
            .unwrap();
        assert!(validate(&path).is_err());
    }

    #[test]
    fn test_merge_identical_independent_runs_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let output = dir.path().join("output");
        fixture(&first, "same");
        snapshot(&first, &second).unwrap();
        let conn = Connection::open(&second).unwrap();
        conn.execute(
            "INSERT INTO runs(source,ingested_at,history_id) SELECT source,ingested_at,lower(hex(randomblob(16))) FROM runs",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO results SELECT 2,test_id,outcome,duration_ms FROM results",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_files SELECT 2,position,path FROM run_files",
            [],
        )
        .unwrap();
        drop(conn);
        merge(&first, &second, &output).unwrap();
        merge(&output, &second, &first).unwrap();
        let conn = Connection::open(&first).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn test_merge_same_identity_conflicting_contents_rejected_without_output_change() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let output = dir.path().join("output");
        fixture(&first, "source");
        snapshot(&first, &second).unwrap();
        Connection::open(&second)
            .unwrap()
            .execute("UPDATE results SET outcome='failed'", [])
            .unwrap();
        std::fs::write(&output, b"unchanged").unwrap();
        assert!(matches!(
            merge(&first, &second, &output),
            Err(SnapshotError::Invalid(
                "one run identity has conflicting contents"
            ))
        ));
        assert_eq!(std::fs::read(output).unwrap(), b"unchanged");
    }

    #[test]
    fn test_merge_two_baseline_clones_keep_baseline_once_and_both_new_runs() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let output = dir.path().join("output");
        fixture(&first, "same");
        snapshot(&first, &second).unwrap();
        for path in [&first, &second] {
            let conn = Connection::open(path).unwrap();
            conn.execute("INSERT INTO runs(source,ingested_at,history_id) SELECT source,ingested_at,lower(hex(randomblob(16))) FROM runs", []).unwrap();
            conn.execute(
                "INSERT INTO results SELECT 2,test_id,outcome,duration_ms FROM results",
                [],
            )
            .unwrap();
        }
        merge(&first, &second, &output).unwrap();
        let conn = Connection::open(&output).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM results", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn test_validate_oversized_file_and_corrupt_bytes_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_DB_BYTES + 1).unwrap();
        assert!(matches!(validate(&path), Err(SnapshotError::Invalid(_))));
        drop(file);
        std::fs::write(&path, b"SQLite format 3\0truncated").unwrap();
        assert!(validate(&path).is_err());
    }

    #[test]
    fn test_snapshot_empty_history_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        drop(Store::open(&input).unwrap());
        snapshot(&input, &output).unwrap();
        let store = Store::open_read_only(&output).unwrap().unwrap();
        assert!(store.history(&[]).unwrap().runs.is_empty());
    }

    #[test]
    fn test_validate_valid_live_wal_input_rejected_without_touching_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fixture(&path, "source");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; UPDATE runs SET source='live-wal';")
            .unwrap();
        let wal = dir.path().join("input-wal");
        let before = std::fs::read(&wal).unwrap();
        assert!(!before.is_empty());
        assert!(matches!(
            validate(&path),
            Err(SnapshotError::Invalid(
                "snapshot must be standalone: SQLite sidecar exists"
            ))
        ));
        assert_eq!(std::fs::read(&wal).unwrap(), before);
        assert_eq!(
            conn.query_row("SELECT source FROM runs", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "live-wal"
        );
    }

    #[test]
    fn test_snapshot_live_wal_output_rejected_preserving_visible_database() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fixture(&input, "replacement");
        fixture(&output, "original");
        let conn = Connection::open(&output).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; UPDATE runs SET source='visible-original';")
            .unwrap();
        let before_db = std::fs::read(&output).unwrap();
        let before_wal = std::fs::read(dir.path().join("output-wal")).unwrap();
        assert!(snapshot(&input, &output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), before_db);
        assert_eq!(
            std::fs::read(dir.path().join("output-wal")).unwrap(),
            before_wal
        );
        let reader = Connection::open(&output).unwrap();
        assert_eq!(
            reader
                .query_row("SELECT source FROM runs", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "visible-original"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_snapshot_nonregular_inputs_and_outputs_rejected_before_sqlite_open() {
        use std::os::unix::fs::{symlink, FileTypeExt};
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let link = dir.path().join("link");
        let socket = dir.path().join("socket");
        fixture(&input, "source");
        symlink(&input, &link).unwrap();
        let _listener = UnixListener::bind(&socket).unwrap();
        for path in [&link, &socket, dir.path()] {
            assert!(matches!(validate(path), Err(SnapshotError::Invalid(_))));
            assert!(snapshot(&input, path).is_err());
        }
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::symlink_metadata(&socket)
            .unwrap()
            .file_type()
            .is_socket());
    }

    #[test]
    fn test_snapshot_orphan_sidecars_are_never_unlinked() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fixture(&input, "source");
        for suffix in ["-wal", "-shm", "-journal"] {
            let sidecar = dir.path().join(format!("output{suffix}"));
            std::fs::write(&sidecar, b"owned-by-another-writer").unwrap();
            assert!(snapshot(&input, &output).is_err());
            assert_eq!(std::fs::read(&sidecar).unwrap(), b"owned-by-another-writer");
            assert!(!output.exists());
            std::fs::remove_file(&sidecar).unwrap();
        }
    }
}

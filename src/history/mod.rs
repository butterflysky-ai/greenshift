//! Durable history snapshots. Git is canonical; a CI cache is an optional copy.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;

use crate::tainted::Tainted;

pub mod snapshot;

/// The portable history protocol's sole reference.
pub const HISTORY_REF: &str = "refs/greenshift/history";
const BLOB: &str = "history.sqlite.gz";
const TIMEOUT: Duration = Duration::from_secs(60);

/// Failure to transport or validate history. Callers decide whether to warn.
#[derive(Debug, thiserror::Error)]
#[error("history: {0}")]
pub struct HistoryError(Tainted);

fn error(message: impl std::fmt::Display) -> HistoryError {
    HistoryError(Tainted::raw(message.to_string()))
}

/// One isolated git transport, with no repository checkout or inherited hooks.
pub struct Remote {
    scratch: TempDir,
    url: String,
    token: Option<SecretString>,
}

impl Remote {
    /// HTTP(S), SSH, and local git remotes are supported. Tokens are optional,
    /// supplied only through GREENSHIFT_HISTORY_TOKEN, never command arguments.
    pub fn new(url: &str) -> Result<Self, HistoryError> {
        if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
            return Err(error("invalid history remote"));
        }
        if url.contains("://")
            && !["https://", "http://", "ssh://", "file://"]
                .iter()
                .any(|prefix| url.starts_with(prefix))
        {
            return Err(error("unsupported history remote protocol"));
        }
        if !url.contains("://") && url.contains(':') {
            return Err(error("use an explicit ssh:// URL for SSH history remotes"));
        }
        // URL-embedded credentials would reach git diagnostics. Require the
        // dedicated secret environment variable instead.
        if let Some((_, authority)) = url.split_once("://") {
            if authority
                .split('/')
                .next()
                .unwrap_or_default()
                .contains('@')
            {
                return Err(error("history remote must not embed credentials"));
            }
        }
        let remote = Self {
            scratch: tempfile::tempdir().map_err(error)?,
            url: url.to_owned(),
            token: std::env::var("GREENSHIFT_HISTORY_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
                .map(SecretString::from),
        };
        remote.git(["init", "--bare", "."], None)?;
        Ok(remote)
    }

    fn git<I, S>(&self, args: I, input: Option<&[u8]>) -> Result<Vec<u8>, HistoryError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let stdout = tempfile::tempfile().map_err(error)?;
        let stderr = tempfile::tempfile().map_err(error)?;
        let mut cmd = Command::new("git");
        // Git's inherited configuration can run hooks, redirect object stores,
        // or install remote helpers. Only the isolated transport owns its state.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(key);
            }
        }
        cmd.current_dir(self.scratch.path())
            .env("HOME", self.scratch.path())
            .env("XDG_CONFIG_HOME", self.scratch.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "greenshift")
            .env("GIT_AUTHOR_EMAIL", "greenshift@localhost")
            .env("GIT_COMMITTER_NAME", "greenshift")
            .env("GIT_COMMITTER_EMAIL", "greenshift@localhost")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "protocol.ext.allow=never",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(stdout.try_clone().map_err(error)?)
            .stderr(stderr.try_clone().map_err(error)?);
        if let Some(token) = &self.token {
            use std::os::unix::fs::PermissionsExt;
            let askpass = self.scratch.path().join("askpass");
            fs::write(&askpass, "#!/bin/sh\ncase \"$1\" in *Username*) echo x-access-token;; *) printf '%s' \"$GREENSHIFT_HISTORY_TOKEN\";; esac\n").map_err(error)?;
            fs::set_permissions(&askpass, fs::Permissions::from_mode(0o700)).map_err(error)?;
            cmd.env("GIT_ASKPASS", askpass)
                .env("GREENSHIFT_HISTORY_TOKEN", token.expose_secret());
        }
        let mut child = cmd.spawn().map_err(error)?;
        if let Some(data) = input {
            if let Err(err) = child
                .stdin
                .take()
                .ok_or_else(|| error("git stdin missing"))?
                .write_all(data)
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(err));
            }
        } else {
            drop(child.stdin.take());
        }
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(error)? {
                break status;
            }
            if started.elapsed() > TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error("git operation timed out"));
            }
            if stdout.metadata().map_err(error)?.len() > snapshot::MAX_DB_BYTES
                || stderr.metadata().map_err(error)?.len() > 1024 * 1024
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error("git output exceeds size limit"));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if !status.success() {
            // Never echo git diagnostics: a remote can reflect credentials.
            return Err(error(format!("git operation failed ({status})")));
        }
        if stdout.metadata().map_err(error)?.len() > snapshot::MAX_DB_BYTES {
            return Err(error("git output exceeds history size limit"));
        }
        use std::io::{Seek, SeekFrom};
        let mut stdout = stdout;
        stdout.seek(SeekFrom::Start(0)).map_err(error)?;
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map_err(error)?;
        Ok(output)
    }

    /// Exact current ref identity, used as the stock cache action's key.
    pub fn head(&self) -> Result<Option<String>, HistoryError> {
        let bytes = self.git(["ls-remote", "--refs", &self.url, HISTORY_REF], None)?;
        let text = std::str::from_utf8(&bytes).map_err(error)?;
        if text.is_empty() {
            return Ok(None);
        }
        let (oid, reference) = text
            .trim_end()
            .split_once('\t')
            .ok_or_else(|| error("invalid remote ref response"))?;
        if reference != HISTORY_REF || !valid_oid(oid) {
            return Err(error("invalid remote history identity"));
        }
        Ok(Some(oid.to_owned()))
    }

    fn fetch(&self, oid: &str) -> Result<(), HistoryError> {
        self.git(["fetch", "--no-tags", "--depth=1", &self.url, oid], None)?;
        Ok(())
    }

    fn read(&self, oid: &str, path: &Path) -> Result<(), HistoryError> {
        self.fetch(oid)?;
        let tree = self.git(["ls-tree", oid], None)?;
        let tree = std::str::from_utf8(&tree).map_err(error)?;
        let parts: Vec<_> = tree.split_whitespace().collect();
        if parts.len() != 4
            || parts[0] != "100644"
            || parts[1] != "blob"
            || parts[3] != BLOB
            || !valid_oid(parts[2])
        {
            return Err(error(
                "history commit must contain exactly one regular snapshot",
            ));
        }
        let bytes = self.git(["cat-file", "blob", parts[2]], None)?;
        decompress(&bytes, path)?;
        snapshot::validate(path).map_err(error)
    }

    fn cached(&self, dir: &Path, oid: &str, stage: &Path) -> Result<(), HistoryError> {
        let directory = dir.join(oid);
        let commit = bounded_read(&directory.join("commit"), 16 * 1024)?;
        let identity = self.git(["hash-object", "-t", "commit", "--stdin"], Some(&commit))?;
        if identity.strip_suffix(b"\n") != Some(oid.as_bytes()) {
            return Err(error("cached commit identity mismatch"));
        }
        let commit = std::str::from_utf8(&commit).map_err(error)?;
        let tree = commit
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("tree "))
            .filter(|tree| valid_oid(tree))
            .ok_or_else(|| error("cached commit has no tree"))?;
        let bytes = bounded_read(&directory.join(BLOB), snapshot::MAX_DB_BYTES)?;
        let blob = self.git(["hash-object", "--stdin"], Some(&bytes))?;
        let blob = std::str::from_utf8(&blob).map_err(error)?.trim();
        let expected = self.git(
            ["mktree", "--missing"],
            Some(format!("100644 blob {blob}\t{BLOB}\n").as_bytes()),
        )?;
        if expected.strip_suffix(b"\n") != Some(tree.as_bytes()) {
            return Err(error("cached snapshot does not match canonical commit"));
        }
        decompress(&bytes, stage)?;
        snapshot::validate(stage).map_err(error)
    }

    /// Restore canonical history, optionally using the exact immutable cache
    /// copy. A missing ref does not treat an arbitrary cached DB as canonical.
    pub fn restore(&self, db: &Path, cache: Option<&Path>) -> Result<bool, HistoryError> {
        let Some(oid) = self.head()? else {
            return Ok(false);
        };
        let parent = db
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(error)?;
        let stage = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
        let hit = cache.is_some_and(|dir| self.cached(dir, &oid, stage.path()).is_ok());
        if !hit {
            self.read(&oid, stage.path())?;
        }
        stage.persist(db).map_err(error)?;
        Ok(true)
    }

    /// Publish validated data, comparing the expected old identity. One rejected
    /// update refetches and merges; a second rejection is returned as a warning
    /// by the CLI. Neither concurrent saves nor caching serialize this operation.
    pub fn publish(&self, artifact: &Path, cache: Option<&Path>) -> Result<String, HistoryError> {
        snapshot::validate(artifact).map_err(error)?;
        let current = self.scratch.path().join("current.sqlite");
        let candidate = self.scratch.path().join("candidate.sqlite");
        let mut old = self.head()?;
        for attempt in 0..2 {
            if let Some(oid) = &old {
                self.read(oid, &current)?;
                snapshot::merge(&current, artifact, &candidate).map_err(error)?;
            } else {
                snapshot::snapshot(artifact, &candidate).map_err(error)?;
            }
            let compressed = compress(&candidate)?;
            let blob = self.git(["hash-object", "-w", "--stdin"], Some(&compressed))?;
            let blob = std::str::from_utf8(&blob).map_err(error)?.trim();
            let tree = self.git(
                ["mktree"],
                Some(format!("100644 blob {blob}\t{BLOB}\n").as_bytes()),
            )?;
            let tree = std::str::from_utf8(&tree).map_err(error)?.trim();
            let commit = self.git(["commit-tree", tree], Some(b"greenshift history\n"))?;
            let oid = std::str::from_utf8(&commit)
                .map_err(error)?
                .trim()
                .to_owned();
            let lease = format!(
                "--force-with-lease={HISTORY_REF}:{}",
                old.as_deref().unwrap_or("")
            );
            let refspec = format!("{oid}:{HISTORY_REF}");
            match self.git(["push", &lease, &self.url, &refspec], None) {
                Ok(_) => {
                    if let Some(dir) = cache {
                        let path = dir.join(&oid);
                        fs::create_dir_all(&path).map_err(error)?;
                        fs::write(path.join(BLOB), compressed).map_err(error)?;
                        let commit = self.git(["cat-file", "commit", &oid], None)?;
                        fs::write(path.join("commit"), commit).map_err(error)?;
                    }
                    return Ok(oid);
                }
                Err(_) if attempt == 0 => old = self.head()?,
                Err(err) => return Err(err),
            }
        }
        Err(error("history update rejected"))
    }
}

fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|b| b.is_ascii_hexdigit())
}

fn bounded_read(path: &Path, cap: u64) -> Result<Vec<u8>, HistoryError> {
    let file = File::open(path).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > cap {
        return Err(error("cached history exceeds size limit"));
    }
    Ok(bytes)
}

fn compress(path: &Path) -> Result<Vec<u8>, HistoryError> {
    let mut reader = File::open(path).map_err(error)?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::copy(&mut reader, &mut encoder).map_err(error)?;
    encoder.finish().map_err(error)
}

fn decompress(bytes: &[u8], path: &Path) -> Result<(), HistoryError> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut bounded = decoder.take(snapshot::MAX_DB_BYTES + 1);
    let mut output = File::create(path).map_err(error)?;
    let len = std::io::copy(&mut bounded, &mut output).map_err(error)?;
    if len > snapshot::MAX_DB_BYTES {
        return Err(error("decompressed snapshot exceeds size limit"));
    }
    output.sync_all().map_err(error)
}

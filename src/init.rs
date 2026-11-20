//! `greenshift init`: set a repository up to run its tests with greenshift
//! in CI, once, with output meant to be committed as is.
//!
//! A backend (only [`nextest`] so far) says which files it wants and with
//! what content ([`Want`]); this module decides what to do with each
//! against what is already on disk ([`decide`]), and then writes, all or
//! nothing: a file that exists and differs is left alone unless `--force`
//! is given, and while one is left alone, none is written. A path
//! through a symlink is never read or written, and every file is staged before any is put
//! in place. A second run with the same inputs finds every file as it
//! would write it and changes nothing.
//!
//! What gets pinned is the greenshift release the workflow downloads,
//! by tag and SHA-256 ([`Release`], [`Sha256`]). Neither has a default: a
//! release is the repository owner's to make and its checksum is to be
//! taken from it, never made up here.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::ValueEnum;

use crate::tainted::Tainted;

pub mod nextest;

/// The last published release without `run nextest` and durable history:
/// a workflow pinned to it, or to anything older, could not work.
pub const LAST_RELEASE_WITHOUT_HISTORY: (u64, u64, u64) = (0, 1, 0);

/// The forges `init` writes a CI workflow for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Forge {
    /// GitHub Actions: a read-only test job and a separate publisher.
    Github,
    /// Forgejo Actions. Not isolated: the workflow says so at its top.
    Forgejo,
}

impl Forge {
    /// The forge's directory in a repository, whose presence says the
    /// repository is on it.
    pub fn dir(self) -> &'static str {
        match self {
            Forge::Github => ".github",
            Forge::Forgejo => ".forgejo",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Forge::Github => "github",
            Forge::Forgejo => "forgejo",
        }
    }

    /// The runner label jobs use unless `--runs-on` is given: GitHub's
    /// hosted Ubuntu runner, and the label a Forgejo runner registers by
    /// default.
    pub fn default_runner(self) -> RunnerLabel {
        RunnerLabel(
            match self {
                Forge::Github => "ubuntu-latest",
                Forge::Forgejo => "docker",
            }
            .to_owned(),
        )
    }
}

/// The forge to write for: `given`, or else the one whose directory
/// ([`Forge::dir`]) `root` has, when exactly one does.
pub fn detect_forge(given: Option<Forge>, root: &Path) -> Result<Forge, String> {
    if let Some(forge) = given {
        return Ok(forge);
    }
    let found: Vec<Forge> = [Forge::Github, Forge::Forgejo]
        .into_iter()
        .filter(|forge| root.join(forge.dir()).is_dir())
        .collect();
    match found[..] {
        [forge] => Ok(forge),
        [] => Err("found neither .github nor .forgejo; say which forge with \
                   --ci github or --ci forgejo"
            .into()),
        _ => Err("found both .github and .forgejo; say which forge with \
                  --ci github or --ci forgejo"
            .into()),
    }
}

/// A greenshift release tag, `vMAJOR.MINOR.PATCH`, as the release
/// workflow publishes and the generated workflow checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    tag: String,
}

impl Release {
    pub fn as_str(&self) -> &str {
        &self.tag
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tag)
    }
}

impl std::str::FromStr for Release {
    type Err = String;

    /// `vMAJOR.MINOR.PATCH`, each a decimal number with no leading zero,
    /// newer than [`LAST_RELEASE_WITHOUT_HISTORY`]. Pre-releases are
    /// refused, as the generated workflow refuses them.
    fn from_str(text: &str) -> Result<Release, String> {
        let shape = || {
            format!(
                "expected a release tag such as v1.2.3, not {}",
                Tainted::raw(text)
            )
        };
        let numbers = text.strip_prefix('v').ok_or_else(shape)?;
        let parts: Vec<&str> = numbers.split('.').collect();
        let [major, minor, patch] = parts[..] else {
            return Err(shape());
        };
        let number = |part: &str| -> Result<u64, String> {
            let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            if !digits || (part.len() > 1 && part.starts_with('0')) {
                return Err(shape());
            }
            part.parse().map_err(|_| shape())
        };
        let version = (number(major)?, number(minor)?, number(patch)?);
        if version <= LAST_RELEASE_WITHOUT_HISTORY {
            let (a, b, c) = LAST_RELEASE_WITHOUT_HISTORY;
            return Err(format!(
                "{text} is too old: v{a}.{b}.{c} and earlier have neither `run nextest` \
                 nor durable history; pin a later release"
            ));
        }
        Ok(Release {
            tag: text.to_owned(),
        })
    }
}

/// A SHA-256 checksum: 64 hex digits, kept in lower case, as
/// `sha256sum` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256(String);

impl Sha256 {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Sha256 {
    type Err = String;

    fn from_str(text: &str) -> Result<Sha256, String> {
        if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Sha256(text.to_ascii_lowercase()))
        } else {
            Err(
                "expected a SHA-256 checksum: 64 hex digits, as the release's \
                 .sha256 file gives it"
                    .into(),
            )
        }
    }
}

/// A runner label for `runs-on`: letters, digits and `.`, `_`, `-`, `:`,
/// so that it goes into a single-quoted YAML string as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerLabel(String);

impl RunnerLabel {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunnerLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for RunnerLabel {
    type Err = String;

    fn from_str(text: &str) -> Result<RunnerLabel, String> {
        let ok = !text.is_empty()
            && text.len() <= 256
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(&b));
        if ok {
            Ok(RunnerLabel(text.to_owned()))
        } else {
            Err(
                "expected a runner label of at most 256 letters, digits, '.', '_', '-' and ':'"
                    .into(),
            )
        }
    }
}

/// A file a backend wants, relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// Exactly this content: written if missing; if present and
    /// different, replaced only with `--force`.
    Generated { path: PathBuf, content: String },
}

/// What `init` does with one wanted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Write `content`; `verb` says how (`wrote`, `overwrote`, ...).
    Write {
        path: PathBuf,
        content: String,
        verb: &'static str,
    },
    /// Nothing to do: already as wanted, for the reason given.
    Unchanged { path: PathBuf, why: String },
    /// The file exists and differs, and `--force` was not given.
    Conflict { path: PathBuf },
}

/// What to do with each of `wants` under `root`, given `force`.
pub fn decide(root: &Path, wants: Vec<Want>, force: bool) -> anyhow::Result<Vec<Action>> {
    wants
        .into_iter()
        .map(|want| match want {
            Want::Generated { path, content } => {
                let existing = read_existing(root, &path)?;
                Ok(match existing {
                    None => Action::Write {
                        path,
                        content,
                        verb: "wrote",
                    },
                    Some(text) if text == content => Action::Unchanged {
                        path,
                        why: "is already up to date".into(),
                    },
                    Some(_) if force => Action::Write {
                        path,
                        content,
                        verb: "overwrote",
                    },
                    Some(_) => Action::Conflict { path },
                })
            }
        })
        .collect()
}

/// The text of the file at `path` under `root`, or `None` if there is
/// none. A symlink at `path`, or on the way to it from `root`, is an
/// error: `init` neither reads what it points to into the repository nor
/// replaces it.
pub fn read_existing(root: &Path, path: &Path) -> anyhow::Result<Option<String>> {
    let mut at = root.to_owned();
    for part in path.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => anyhow::bail!(
                "{} is a symlink; left alone, and nothing written",
                Tainted::from(at.strip_prefix(root).unwrap_or(&at))
            ),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => {
                return Err(err).with_context(|| format!("could not read {}", Tainted::from(path)))
            }
        }
    }
    let full = root.join(path);
    let path = full.as_path();
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .with_context(|| format!("{} is not UTF-8; left alone", Tainted::from(path))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("could not read {}", Tainted::from(path))),
    }
}

/// Carry out `actions` under `root`, saying what happened to each on
/// stdout, and return the exit status: 0, or 1 with nothing written when
/// any is a [`Action::Conflict`], each named on stderr.
pub fn apply(root: &Path, actions: &[Action]) -> anyhow::Result<u8> {
    let conflicts: Vec<&Path> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Conflict { path } => Some(path.as_path()),
            _ => None,
        })
        .collect();
    if !conflicts.is_empty() {
        for path in &conflicts {
            eprintln!(
                "greenshift: {} exists and differs from what init writes; \
                 left alone (--force overwrites it)",
                Tainted::from(*path)
            );
        }
        eprintln!("greenshift: nothing written");
        return Ok(1);
    }
    // Stage every file first, so that one that cannot be written leaves
    // every other one as it was.
    let mut staged = Vec::new();
    for action in actions {
        if let Action::Write { path, content, .. } = action {
            let file = stage(&root.join(path), content).with_context(|| {
                format!(
                    "could not write {}; nothing written",
                    Tainted::from(path.as_path())
                )
            })?;
            staged.push(file);
        }
    }
    let mut staged = staged.into_iter();
    let mut changed = false;
    for action in actions {
        match action {
            Action::Write { path, verb, .. } => {
                let file = staged.next().expect("one staged file per write");
                file.persist(root.join(path))
                    .map_err(|err| err.error)
                    .with_context(|| {
                        format!("could not write {}", Tainted::from(path.as_path()))
                    })?;
                println!("greenshift: {verb} {}", Tainted::from(path.as_path()));
                changed = true;
            }
            Action::Unchanged { path, why } => {
                println!("greenshift: {} {why}", Tainted::from(path.as_path()));
            }
            Action::Conflict { .. } => unreachable!("conflicts return above"),
        }
    }
    if !changed {
        println!("greenshift: nothing to change; the repository is already set up");
    }
    Ok(0)
}

/// `content` in a temporary file beside `path`, creating its directory,
/// for renaming into place, so the file is never left half written. It
/// has the mode of the file it replaces, or else the mode any new file
/// gets.
fn stage(path: &Path, content: &str) -> std::io::Result<tempfile::NamedTempFile> {
    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    let dir = dir.unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".greenshift-init-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Subject to the umask, as for any new file.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let mut file = builder.tempfile_in(dir)?;
    match std::fs::metadata(path) {
        Ok(meta) => file.as_file().set_permissions(meta.permissions())?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::io::Write::write_all(&mut file, content.as_bytes())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_are_plain_tags_newer_than_the_last_without_history() {
        for ok in ["v0.1.1", "v0.2.0", "v1.0.0", "v10.20.30"] {
            assert_eq!(ok.parse::<Release>().unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "v",
            "0.2.0",
            "v0.2",
            "v0.2.0.1",
            "v0.2.0-rc.1",
            "v01.2.0",
            "v0.02.0",
            "v+1.2.3",
            "V0.2.0",
            "v0.2.0 ",
            "v0.1.0",
            "v0.0.1",
        ] {
            assert!(bad.parse::<Release>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn checksums_are_64_hex_digits_kept_in_lower_case() {
        let upper = "ABCDEF0123456789".repeat(4);
        assert_eq!(
            upper.parse::<Sha256>().unwrap().as_str(),
            upper.to_ascii_lowercase()
        );
        for bad in [
            String::new(),
            "a".repeat(63),
            "a".repeat(65),
            "z".repeat(64),
        ] {
            assert!(bad.parse::<Sha256>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn runner_labels_go_into_yaml_as_they_are() {
        for ok in ["ubuntu-24.04", "self-hosted:docker_x", "-", "null"] {
            assert_eq!(ok.parse::<RunnerLabel>().unwrap().as_str(), ok);
        }
        let long = "a".repeat(257);
        for bad in ["", "a b", "a\nb", "'x'", "x#y", "[x]", &long] {
            assert!(bad.parse::<RunnerLabel>().is_err(), "{bad:?}");
        }
        assert!("a".repeat(256).parse::<RunnerLabel>().is_ok());
    }
}

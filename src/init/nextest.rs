//! `greenshift init nextest`: the files a cargo-nextest repository needs
//! to run its tests with `greenshift run nextest` in CI, keeping the
//! durable history as docs/design/durable-history.md designs it.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, TableLike};

use super::{Forge, MergeOutcome, Release, Sha256, Want};

/// The cargo-nextest version the workflow installs, as the durable-history
/// reference workflow pins it.
pub const NEXTEST_VERSION: &str = "0.9.143";

/// The rustup the Forgejo workflow installs Rust with when the runner has
/// none, and the SHA-256 of its x86_64-unknown-linux-gnu `rustup-init`, as
/// static.rust-lang.org publishes it (checked against the download).
pub const RUSTUP_VERSION: &str = "1.29.3";
pub const RUSTUP_SHA256: &str = "b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55";

/// What the workflow is made from.
#[derive(Debug, Clone)]
pub struct Settings<'a> {
    pub forge: Forge,
    pub release: &'a Release,
    pub sha256: &'a Sha256,
    /// The runner label every job runs on.
    pub runs_on: &'a str,
}

/// Where the workflow goes for `forge`.
pub fn workflow_path(forge: Forge) -> PathBuf {
    Path::new(forge.dir()).join("workflows/greenshift.yml")
}

/// The nextest config file, relative to the repository root.
pub const NEXTEST_TOML: &str = ".config/nextest.toml";

/// The JUnit path the stanza gives the default profile, relative to the
/// profile's store directory (`target/nextest/default/`).
pub const JUNIT_PATH: &str = "junit.xml";

/// The comment above an added stanza.
const STANZA_COMMENT: &str = "# JUnit output, for `greenshift ingest` and other JUnit readers.\n\
                              # Added by `greenshift init nextest`.\n";

/// The files `init nextest` wants, in the order it reports them.
pub fn wants(root: &Path, settings: &Settings<'_>) -> anyhow::Result<Vec<Want>> {
    let existing = super::read_existing(&root.join(NEXTEST_TOML))?;
    let outcome = junit_stanza(existing.as_deref())
        .map_err(|why| anyhow::anyhow!("{NEXTEST_TOML} {why}; left alone"))?;
    Ok(vec![
        Want::Generated {
            path: workflow_path(settings.forge),
            content: workflow(settings),
        },
        Want::Merged {
            path: NEXTEST_TOML.into(),
            outcome,
        },
    ])
}

/// What becomes of `.config/nextest.toml`, `existing` being its text if
/// there is one: the default profile's JUnit output turned on, unless the
/// file already sets a path for it. Everything else in the file is kept as
/// it is, comments and layout included. Text that is not TOML, or a
/// setting on the way to the path that is not a table, is an error: the
/// file is nextest's and its owner's to fix.
pub fn junit_stanza(existing: Option<&str>) -> Result<MergeOutcome, String> {
    let Some(text) = existing.filter(|text| !text.is_empty()) else {
        return Ok(MergeOutcome::Write {
            content: format!("{STANZA_COMMENT}[profile.default.junit]\npath = \"{JUNIT_PATH}\"\n"),
            what: if existing.is_some() {
                "added the JUnit stanza to"
            } else {
                "wrote"
            }
            .into(),
        });
    };
    let mut doc = text
        .parse::<DocumentMut>()
        .map_err(|err| format!("is not TOML: {}", one_line(&err.to_string())))?;
    let profile = child_table(doc.as_table_mut(), "profile", "profile")?;
    let default = child_table(profile, "default", "profile.default")?;
    let junit = child_table(default, "junit", "profile.default.junit")?;
    if let Some(path) = junit.get("path") {
        return match path.as_str() {
            Some(path) => Ok(MergeOutcome::Keep {
                why: format!(
                    "already sets the default profile's JUnit path ({}); left alone",
                    crate::tainted::Tainted::raw(path)
                ),
            }),
            None => Err("sets profile.default.junit.path to something other than a string".into()),
        };
    }
    junit.insert("path", toml_edit::value(JUNIT_PATH));
    Ok(MergeOutcome::Write {
        content: doc.to_string(),
        what: "added the JUnit stanza to".into(),
    })
}

/// The table at `key` in `parent`, made if missing, or an error naming
/// `dotted` if something else is there.
fn child_table<'t>(
    parent: &'t mut dyn TableLike,
    key: &str,
    dotted: &str,
) -> Result<&'t mut dyn TableLike, String> {
    if parent.get(key).is_none() {
        let mut table = Table::new();
        if key == "junit" {
            table.decor_mut().set_prefix(format!("\n{STANZA_COMMENT}"));
        } else {
            table.set_implicit(true);
        }
        parent.insert(key, Item::Table(table));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| format!("sets {dotted} to something other than a table"))
}

/// `text` on one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The CI workflow for `settings`.
pub fn workflow(settings: &Settings<'_>) -> String {
    let template = match settings.forge {
        Forge::Github => include_str!("nextest/github.yml"),
        Forge::Forgejo => include_str!("nextest/forgejo.yml"),
    };
    fill(template, settings)
}

/// `template` with its `@NAME@` placeholders filled in.
fn fill(template: &str, settings: &Settings<'_>) -> String {
    let install = indent(include_str!("nextest/install.sh"), 10);
    let install_rust = indent(include_str!("nextest/install-rust.sh"), 10);
    template
        .replace("@INSTALL@\n", &install)
        .replace("@INSTALL_RUST@\n", &install_rust)
        .replace("@RUSTUP@", RUSTUP_VERSION)
        .replace("@RUSTUP_SHA256@", RUSTUP_SHA256)
        .replace("@RELEASE@", settings.release.as_str())
        .replace("@SHA256@", settings.sha256.as_str())
        .replace("@RUNS_ON@", settings.runs_on)
        .replace("@NEXTEST@", NEXTEST_VERSION)
}

/// Every line of `text` indented by `by` spaces.
fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines()
        .map(|line| {
            if line.is_empty() {
                "\n".to_owned()
            } else {
                format!("{pad}{line}\n")
            }
        })
        .collect()
}

/// The placeholders [`fill`] fills.
#[cfg(test)]
const PLACEHOLDERS: &[&str] = &[
    "@INSTALL@",
    "@RELEASE@",
    "@SHA256@",
    "@RUNS_ON@",
    "@NEXTEST@",
    "@INSTALL_RUST@",
    "@RUSTUP@",
    "@RUSTUP_SHA256@",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled() {
        let release: Release = "v0.2.0".parse().unwrap();
        let sha256: Sha256 = "a".repeat(64).parse().unwrap();
        for forge in [Forge::Github, Forge::Forgejo] {
            let settings = Settings {
                forge,
                release: &release,
                sha256: &sha256,
                runs_on: forge.default_runner(),
            };
            let text = workflow(&settings);
            for placeholder in PLACEHOLDERS {
                assert!(!text.contains(placeholder), "{forge:?}: {placeholder}");
            }
        }
    }
}

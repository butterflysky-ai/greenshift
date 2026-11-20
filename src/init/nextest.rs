//! `greenshift init nextest`: the files a cargo-nextest repository needs
//! to run its tests with `greenshift run nextest` in CI, keeping the
//! durable history as docs/design/durable-history.md designs it.

use std::path::{Path, PathBuf};

use super::{Forge, Release, Sha256, Want};

/// The cargo-nextest version the workflow installs, as the durable-history
/// reference workflow pins it.
pub const NEXTEST_VERSION: &str = "0.9.143";

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

/// The files `init nextest` wants, in the order it reports them.
pub fn wants(_root: &Path, settings: &Settings<'_>) -> anyhow::Result<Vec<Want>> {
    Ok(vec![Want::Generated {
        path: workflow_path(settings.forge),
        content: workflow(settings),
    }])
}

/// The CI workflow for `settings`.
pub fn workflow(settings: &Settings<'_>) -> String {
    let template = match settings.forge {
        Forge::Github => include_str!("nextest/github.yml"),
        Forge::Forgejo => unimplemented!("forgejo"),
    };
    fill(template, settings)
}

/// `template` with its `@NAME@` placeholders filled in.
fn fill(template: &str, settings: &Settings<'_>) -> String {
    let install = indent(include_str!("nextest/install.sh"), 10);
    template
        .replace("@INSTALL@\n", &install)
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
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled() {
        let release: Release = "v0.2.0".parse().unwrap();
        let sha256: Sha256 = "a".repeat(64).parse().unwrap();
        for forge in [Forge::Github] {
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

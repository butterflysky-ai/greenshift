//! `greenshift init nextest`: the files a cargo-nextest repository needs
//! to run its tests with `greenshift run nextest` in CI, keeping the
//! durable history as docs/design/durable-history.md designs it.

use std::path::{Path, PathBuf};

use super::{Forge, Release, RunnerLabel, Sha256, Want};

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
    pub runs_on: &'a RunnerLabel,
}

/// Where the workflow goes for `forge`.
pub fn workflow_path(forge: Forge) -> PathBuf {
    Path::new(forge.dir()).join("workflows/greenshift.yml")
}

/// The files `init nextest` wants, in the order it reports them: only the
/// workflow. `run nextest` has nextest write JUnit to files of its own, so
/// the repository's `.config/nextest.toml` needs nothing and is left alone.
pub fn wants(settings: &Settings<'_>) -> Vec<Want> {
    vec![Want::Generated {
        path: workflow_path(settings.forge),
        content: workflow(settings),
    }]
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
        .replace("@RUNS_ON@", settings.runs_on.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled() {
        let release: Release = "v0.2.0".parse().unwrap();
        let sha256: Sha256 = "a".repeat(64).parse().unwrap();
        for forge in [Forge::Github, Forge::Forgejo] {
            let runs_on = forge.default_runner();
            let settings = Settings {
                forge,
                release: &release,
                sha256: &sha256,
                runs_on: &runs_on,
            };
            let text = workflow(&settings);
            // Any `@NAME@` left, whether or not `fill` knows the name.
            let parts: Vec<&str> = text.split('@').collect();
            let between = &parts[1..parts.len().saturating_sub(1).max(1)];
            let left: Vec<&&str> = between
                .iter()
                .filter(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                })
                .collect();
            assert!(left.is_empty(), "{forge:?}: {left:?}");
        }
    }
}

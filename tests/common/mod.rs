use std::path::Path;
use std::process::{Command, Output};

/// Compile real producer modules in an isolated fixture, optionally running it
/// while its temporary source and executable still exist.
pub fn compile(execute: bool, fixture: impl FnOnce(&Path) -> String) -> (Output, Option<Output>) {
    let directory = tempfile::tempdir().expect("create contract fixture directory");
    let source = directory.path().join("contract.rs");
    let binary = directory.path().join("contract");
    std::fs::write(&source, fixture(directory.path())).expect("write contract fixture");
    let compilation = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("run rustc for contract fixture");
    let execution = (execute && compilation.status.success()).then(|| {
        Command::new(&binary)
            .output()
            .expect("execute compiled contract fixture")
    });
    (compilation, execution)
}

use std::process::ExitCode;

fn main() -> ExitCode {
    match greenshift::cli::run() {
        Ok(code) => code,
        Err(err) => {
            greenshift::cli::show_error(&err);
            ExitCode::FAILURE
        }
    }
}

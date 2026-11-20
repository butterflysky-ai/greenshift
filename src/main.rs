use std::process::ExitCode;

fn main() -> ExitCode {
    let control = greenshift::cli::TransportControl::default();
    let interrupted = control.clone();
    if let Err(error) = ctrlc::set_handler(move || {
        if !interrupted.interrupt() {
            std::process::exit(130);
        }
    }) {
        eprintln!("greenshift: could not install interrupt handler: {error}");
        return ExitCode::FAILURE;
    }
    match greenshift::cli::run_with_control(&control) {
        Ok(code) => code,
        Err(err) => {
            greenshift::cli::show_error(&err);
            ExitCode::FAILURE
        }
    }
}

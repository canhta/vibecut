//! Isolated Gipformer recognition process entry point.
fn main() -> std::process::ExitCode {
    match ai::recognition::gipformer::run_native() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}

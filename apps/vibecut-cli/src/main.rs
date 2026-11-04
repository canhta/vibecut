//! Process boundary: strip the JSON switch, dispatch, print, map the typed exit code.
use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<OsString> = env::args_os().skip(1).collect();
    let json = args.iter().any(|arg| arg == "--json");
    args.retain(|arg| arg != "--json");
    match vibecut_cli::run(args) {
        Ok(output) => match output.write(json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error.report(json);
                ExitCode::from(error.exit_code())
            }
        },
        Err(error) => {
            error.report(json);
            ExitCode::from(error.exit_code())
        }
    }
}

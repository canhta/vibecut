//! Adapts explicit font requests to the libraries owner.
use crate::args::CliError;
use crate::args::{one_utf8, utf8};
use crate::output::Output;
use crate::output::wait_for_job;
use crate::session::{Context, Lent, cli_runner, run_command, standalone_session, with_fonts};
use engine::control::{ControlResult, FontLoadState};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io;
use std::io::Write as _;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    const USAGE: &str = "usage: vibecut-cli fonts list [--query <text>] | resolve <family> | download <family> --accept-license | import <file> --accept-license";
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| CliError::Usage(USAGE.into()))?;
    let action = utf8(&action, "fonts action")?.to_owned();
    let rest: Vec<OsString> = args.collect();
    match action.as_str() {
        "list" => {
            let mut query = None;
            let mut rest = rest.into_iter();
            while let Some(arg) = rest.next() {
                match utf8(&arg, "fonts list flag")? {
                    "--query" if query.is_none() => {
                        let value = rest.next().ok_or_else(|| CliError::Usage(USAGE.into()))?;
                        query = Some(utf8(&value, "fonts query")?.to_owned());
                    }
                    other => {
                        return Err(CliError::Usage(format!("unknown fonts list flag: {other}")));
                    }
                }
            }
            let entries = match fonts_call(context, "fonts.list", json!({"query": query}))? {
                ControlResult::Fonts { entries } => entries,
                _ => return Err(CliError::Rejected("fonts.list returned no listing")),
            };
            let human = entries
                .iter()
                .map(|entry| {
                    format!(
                        "{}\t{}\t{}\t{}",
                        entry.family,
                        json_name(&entry.origin),
                        json_name(&entry.state),
                        entry.styles.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Output::new(json!({"fonts": entries}), human, false, 0))
        }
        "resolve" => {
            let family = one_utf8(rest, "usage: vibecut-cli fonts resolve <family>")?;
            let font = match fonts_call(context, "fonts.load", json!({"family": family}))? {
                ControlResult::Font { font } => font,
                _ => return Err(CliError::Rejected("fonts.load returned no font")),
            };
            let state = json_name(&font.state);
            let human = match font.state {
                FontLoadState::Loaded => format!("{}: loaded", font.family),
                FontLoadState::NotDownloaded | FontLoadState::CacheInvalid => format!(
                    "{}: {state}; layout uses {}. Run `vibecut-cli fonts download {} --accept-license`.",
                    font.family, font.effective_family, font.family
                ),
                FontLoadState::Unknown => format!(
                    "{}: unknown family; layout uses {}",
                    font.family, font.effective_family
                ),
            };
            Ok(Output::new(
                serde_json::to_value(&font)?,
                human,
                false,
                if font.state == FontLoadState::Loaded {
                    0
                } else {
                    4
                },
            ))
        }
        "download" => fonts_download(context, rest),
        "import" => {
            let mut args = rest.into_iter();
            let path = args.next().ok_or_else(|| {
                CliError::Usage("usage: vibecut-cli fonts import <file> --accept-license".into())
            })?;
            let mut accepted = false;
            for flag in args {
                if flag == "--accept-license" {
                    accepted = true;
                } else {
                    return Err(CliError::Usage("unknown font import flag".into()));
                }
            }
            fonts_job(
                context,
                "fonts.import",
                json!({"path":std::path::PathBuf::from(path),"accept_license":accepted}),
            )?;
            Ok(Output::new(
                json!({"status":"imported"}),
                "imported custom font".into(),
                false,
                0,
            ))
        }
        other => Err(CliError::Usage(format!(
            "unknown fonts action: {other}; expected list, resolve, download or import"
        ))),
    }
}

fn fonts_download(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    const USAGE: &str = "usage: vibecut-cli fonts download <family> --accept-license";
    let mut args = args.into_iter();
    let family = args.next().ok_or_else(|| CliError::Usage(USAGE.into()))?;
    let family = utf8(&family, "font family")?.to_owned();
    let mut accepted = false;
    for arg in args {
        match utf8(&arg, "fonts download flag")? {
            "--accept-license" => accepted = true,
            other => {
                return Err(CliError::Usage(format!(
                    "unknown fonts download flag: {other}"
                )));
            }
        }
    }
    let entries = match fonts_call(context, "fonts.list", json!({"query": family}))? {
        ControlResult::Fonts { entries } => entries,
        _ => return Err(CliError::Rejected("fonts.list returned no listing")),
    };
    let entry = entries
        .into_iter()
        .find(|entry| {
            entry.family.eq_ignore_ascii_case(family.trim())
                && entry.origin == engine::control::FontOrigin::Google
        })
        .ok_or(CliError::Rejected("not a downloadable catalog font"))?;
    let _ = writeln!(
        io::stderr(),
        "family: {}\nlicence: {}\nsource: {}\nbytes: {}",
        entry.family,
        entry.license.as_deref().unwrap_or("unknown"),
        entry.source.as_deref().unwrap_or("unknown"),
        entry.download_bytes.unwrap_or(0)
    );
    if !accepted {
        let _ = writeln!(
            io::stderr(),
            "download: refused; pass --accept-license to accept the source and licence"
        );
        return Err(CliError::Rejected(
            "font download requires --accept-license after reviewing licence and source",
        ));
    }
    fonts_job(
        context,
        "fonts.download",
        json!({"family":entry.family,"accept_license":accepted}),
    )?;
    Ok(Output::new(
        json!({"family": entry.family, "status": "downloaded"}),
        format!("downloaded {}", entry.family),
        false,
        0,
    ))
}

/// Submit font work on a standalone runner and wait on that same runner so the
/// job's publication fence survives to completion.
fn fonts_job(context: &Context, command: &str, params: Value) -> Result<(), CliError> {
    let mut session = standalone_session()?;
    let mut runner = cli_runner(context.budget()?)?;
    let result = with_fonts(|fonts| {
        match run_command(
            &mut session,
            &mut runner,
            Lent {
                fonts,
                ..Lent::default()
            },
            command,
            params,
            true,
        )? {
            ControlResult::JobAccepted { job, .. } => {
                wait_for_job(&mut runner, job)?;
                Ok(())
            }
            _ => Err(CliError::Rejected("font operation did not start a job")),
        }
    });
    runner.shutdown();
    result
}

/// Run one font command in a standalone session with the per-user store lent.
fn fonts_call(context: &Context, command: &str, params: Value) -> Result<ControlResult, CliError> {
    let mut session = standalone_session()?;
    let mut runner = cli_runner(context.budget()?)?;
    let result = with_fonts(|fonts| {
        run_command(
            &mut session,
            &mut runner,
            Lent {
                fonts,
                ..Lent::default()
            },
            command,
            params,
            false,
        )
    });
    runner.shutdown();
    result
}

fn json_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

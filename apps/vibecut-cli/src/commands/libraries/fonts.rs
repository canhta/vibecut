//! Adapts explicit font requests to the libraries owner.
use crate::args::CliError;
use crate::args::{one_utf8, utf8};
use crate::output::Output;
use crate::output::wait_for_job;
use crate::session::Context;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use libraries::fonts::{FontLoadState, FontStore};
use serde_json::json;
use std::ffi::OsString;
use std::io;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    const USAGE: &str = "usage: vibecut-cli fonts list [--query <text>] | resolve <family> | download <family> --accept-license";
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
            let entries = FontStore::native()?.list(query.as_deref())?;
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
            let font = FontStore::native()?.load(&family)?;
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
        other => Err(CliError::Usage(format!(
            "unknown fonts action: {other}; expected list, resolve or download"
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
    let store = FontStore::native()?;
    let entry = store
        .list(Some(&family))?
        .into_iter()
        .find(|entry| {
            entry.family.eq_ignore_ascii_case(family.trim())
                && entry.origin == libraries::fonts::FontOrigin::Google
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
    let budget = context.budget()?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 3,
            backoff: Duration::from_millis(500),
            progress_interval: Duration::from_millis(100),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?;
    let (id, _) = store.submit_download(&mut runner, &family, true)?;
    wait_for_job(&mut runner, id)?;
    runner.shutdown();
    Ok(Output::new(
        json!({"family": entry.family, "status": "downloaded"}),
        format!("downloaded {}", entry.family),
        false,
        0,
    ))
}
fn json_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

//! Adapts explicit model transfers to engine jobs.
use crate::args::CliError;
use crate::args::{one_utf8, utf8};
use crate::output::Output;
use crate::output::wait_for_job;
use crate::session::Context;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use engine::subtitles;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args
        .next()
        .ok_or_else(|| CliError::Usage("usage: vibecut-cli model list|info|download".into()))?;
    let action = utf8(&action, "model action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "list" => {
            if !rest.is_empty() {
                return Err(CliError::Usage("usage: vibecut-cli model list".into()));
            }
            let models = subtitles::model_infos()?;
            let value: Vec<Value> = models.iter().map(model_json).collect();
            let human = models
                .iter()
                .map(|model| {
                    format!(
                        "{}\t{}\t{} files\t{} bytes\t{}",
                        model.id,
                        model.license,
                        model.files.len(),
                        model.total_bytes,
                        model.source
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Output::new(json!({"models": value}), human, false, 0))
        }
        "info" => {
            let id = one_utf8(rest, "usage: vibecut-cli model info <id>")?;
            let model = subtitles::model_info(&id)?;
            Ok(Output::new(
                model_json(&model),
                model_human(&model),
                false,
                0,
            ))
        }
        "download" => model_download(context, rest),
        other => Err(CliError::Usage(format!(
            "unknown model action: {other}; expected list, info or download"
        ))),
    }
}
fn model_download(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let id = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli model download <id> --yes-download".into())
    })?;
    let id = utf8(&id, "model id")?.to_owned();
    let mut accepted = false;
    for arg in args {
        match utf8(&arg, "model download flag")? {
            "--yes-download" => accepted = true,
            other => {
                return Err(CliError::Usage(format!(
                    "unknown model download flag: {other}"
                )));
            }
        }
    }
    let info = subtitles::model_info(&id)?;
    let _ = writeln!(io::stderr(), "{}", model_human(&info));
    if !accepted {
        let _ = writeln!(
            io::stderr(),
            "download: refused; pass --yes-download to accept the source and licence"
        );
        return Err(CliError::Rejected(
            "model download requires --yes-download after reviewing licence and source",
        ));
    }
    let budget = context.budget()?;
    let downloader = subtitles::ModelDownloader::open(budget.clone())?;
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
    let id = downloader.submit(&mut runner, &id)?;
    wait_for_job(&mut runner, id)?;
    runner.shutdown();
    Ok(Output::new(
        json!({"model": info.id, "status": "installed"}),
        format!("installed {}", info.id),
        false,
        0,
    ))
}
fn model_json(model: &subtitles::ModelInfo) -> Value {
    json!({
        "id": model.id,
        "license": model.license,
        "license_url": model.license_url,
        "source": model.source,
        "languages": model.languages,
        "total_bytes": model.total_bytes,
        "files": model.files.iter().map(|file| json!({
            "name": file.name,
            "source": file.source,
            "sha256": file.sha256,
            "size": file.size,
        })).collect::<Vec<_>>(),
    })
}
fn model_human(model: &subtitles::ModelInfo) -> String {
    let mut human = format!(
        "id: {}\nlicence: {} ({})\nsource: {}\nlanguages: {}\nbytes: {}\nfiles:\n",
        model.id,
        model.license,
        model.license_url,
        model.source,
        model.languages.join(", "),
        model.total_bytes
    );
    for file in &model.files {
        let _ = writeln!(
            human,
            "  {} {} bytes sha256 {}\n    {}",
            file.name, file.size, file.sha256, file.source
        );
    }
    human.trim_end().to_owned()
}

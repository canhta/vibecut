//! Classifies filler words in one project transcript through the lent text provider.
use crate::args::{CliError, utf8};
use crate::output::{CliApproval, Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner};
use engine::fillers::FillerRequest;
use engine::providers::{Credentials, TextAccess, choose_filler};
use project::WriterLease;
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(project) = args.next() else {
        return Err(usage());
    };
    if project
        .to_str()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        return Ok(usage_output());
    }
    let project_path = PathBuf::from(project);
    let mut language: Option<String> = None;
    let mut include_markers = false;
    let mut llm = false;
    let mut provider = "openai".to_owned();
    let mut model: Option<String> = None;
    let mut yes_upload = false;
    while let Some(flag) = args.next() {
        match utf8(&flag, "fillers flag")? {
            "--language" => {
                language = Some(
                    utf8(
                        &args.next().ok_or_else(|| {
                            CliError::Usage("--language requires vi, en, zh or a zh variant".into())
                        })?,
                        "transcript language",
                    )?
                    .to_owned(),
                );
            }
            "--include-markers" => include_markers = true,
            "--llm" => llm = true,
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "filler provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "filler model",
                    )?
                    .to_owned(),
                );
            }
            "--yes-upload" => yes_upload = true,
            "--help" | "-h" => return Ok(usage_output()),
            other => return Err(CliError::Usage(format!("unknown fillers flag: {other}"))),
        }
    }
    if !llm {
        return Err(CliError::Usage(
            "fillers requires --llm; local candidates use `exec timeline.filler_candidates`".into(),
        ));
    }
    let language =
        language.ok_or_else(|| CliError::Usage("fillers requires --language <tag>".into()))?;
    let language = engine::providers::parse_language(&language)?;
    if !ai::supports_text_language(&language) {
        return Err(CliError::Usage(
            "--language must be vi, en, zh, zh-Hans or zh-Hant".into(),
        ));
    }
    let provider = choose_filler(&provider, model.as_deref())?;
    let credentials = Credentials::new(
        std::env::var("OPENAI_API_KEY").ok(),
        std::env::var("ELEVENLABS_API_KEY").ok(),
        std::env::var("ANTHROPIC_API_KEY").ok(),
    )?;
    let access = TextAccess::open(
        provider,
        &credentials,
        Box::new(CliApproval::new(yes_upload)),
    )?;

    let lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let project = std::sync::Arc::new(loaded.project);
    let budget = context.budget()?;
    let mut runner = cli_runner(budget)?;
    let request = FillerRequest::new(&project, language, include_markers)?;
    let job = request.submit(&mut runner, &access)?;
    wait_for_job(&mut runner, job.id)?;
    let candidates = job.finish(&mut runner)?.ok_or(CliError::Rejected(
        "filler classification produced no result",
    ))?;
    runner.shutdown();

    let mut human = String::new();
    for candidate in &candidates {
        let _ = writeln!(
            human,
            "{:.3} {:.3} {} {:?}",
            candidate.range.start.seconds(),
            candidate.range.end.seconds(),
            candidate.id.as_str(),
            candidate.kind
        );
    }
    let _ = write!(human, "{} filler candidates", candidates.len());
    let value = json!({
        "status": "completed",
        "project": json_path(&project_path)?,
        "provider": access.filler_provider().id().as_str(),
        "model": access.filler_provider().model().as_str(),
        "language": request.input().settings.language.as_str(),
        "include_markers": include_markers,
        "candidates": candidates,
    });
    Ok(Output::new(value, human, false, 0))
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}
fn usage_text() -> &'static str {
    "usage: vibecut-cli fillers <project> --language vi|en|zh|zh-Hans|zh-Hant --llm \
     [--include-markers] [--provider openai|anthropic] [--model <id>] [--yes-upload]"
}
fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}

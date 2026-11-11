//! Generates chapters, title and description for one project transcript.
use crate::args::{CliError, parse_u64, utf8};
use crate::output::{CliConsent, Output, json_path, wait_for_job};
use crate::session::{Context, cli_runner};
use engine::providers::{Credentials, TextAccess, choose_chapter};
use engine::{chapter_sidecars, chapters};
use project::WriterLease;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

const DEFAULT_MAX_CHAPTERS: usize = 10;

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
    let mut max_chapters = DEFAULT_MAX_CHAPTERS;
    let mut provider = "openai".to_owned();
    let mut model: Option<String> = None;
    let mut export: Option<PathBuf> = None;
    let mut yes_upload = false;
    let mut accept_unknown_price = false;
    while let Some(flag) = args.next() {
        match utf8(&flag, "chapters flag")? {
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
            "--max-chapters" => {
                max_chapters = usize::try_from(parse_u64(args.next(), "--max-chapters")?)
                    .map_err(|_| CliError::Usage("--max-chapters is too large".into()))?;
            }
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "chapter provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "chapter model",
                    )?
                    .to_owned(),
                );
            }
            "--export" => {
                export = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--export requires a directory".into())
                })?));
            }
            "--yes-upload" => yes_upload = true,
            "--accept-unknown-price" => accept_unknown_price = true,
            "--help" | "-h" => return Ok(usage_output()),
            other => {
                return Err(CliError::Usage(format!("unknown chapters flag: {other}")));
            }
        }
    }
    let language =
        language.ok_or_else(|| CliError::Usage("chapters requires --language <tag>".into()))?;
    let language = engine::providers::parse_language(&language)?;
    if !ai::supports_text_language(&language) {
        return Err(CliError::Usage(
            "--language must be vi, en, zh, zh-Hans or zh-Hant".into(),
        ));
    }
    let provider = choose_chapter(&provider, model.as_deref())?;
    let credentials = Credentials::from_env();
    let access = TextAccess::open(
        provider,
        &credentials,
        Box::new(CliConsent::new(yes_upload, accept_unknown_price)),
    )?;

    let lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let project = Arc::new(loaded.project);
    let budget = context.budget()?;
    let mut runner = cli_runner(budget)?;
    let request = chapters::ChapterRequest::new(&project, language, max_chapters)?;
    let job = request.submit(&mut runner, &access)?;
    wait_for_job(&mut runner, job.id)?;
    let document = job.finish(&mut runner)?.ok_or(CliError::Rejected(
        "chapter generation produced no document",
    ))?;
    runner.shutdown();

    let sidecars = match export {
        Some(directory) => {
            let protected = vec![project_path.clone(), project::metadata_path(&project_path)?];
            Some(chapter_sidecars::write_to_directory(
                &project, &protected, &directory, &document,
            )?)
        }
        None => None,
    };
    let human = document
        .to_text()
        .map_err(|error| CliError::Engine(engine::control::Error::Ai(error)))?;
    let value = response_json(&project_path, &access, &document, sidecars.as_ref())?;
    Ok(Output::new(value, human, false, 0))
}

fn response_json(
    project: &std::path::Path,
    access: &TextAccess,
    document: &ai::chapters::ChapterDocument,
    sidecars: Option<&chapter_sidecars::ChapterSidecars>,
) -> Result<Value, CliError> {
    Ok(json!({
        "status": "completed",
        "project": json_path(project)?,
        "provider": access.chapter_provider().id().as_str(),
        "model": access.chapter_provider().model().as_str(),
        "language": document.language().as_str(),
        "document": serde_json::to_value(document)?,
        "sidecars": sidecars.map(serde_json::to_value).transpose()?,
    }))
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}
fn usage_text() -> &'static str {
    "usage: vibecut-cli chapters <project> --language vi|en|zh|zh-Hans|zh-Hant \
     [--max-chapters 10] [--provider openai|anthropic] [--model <id>] \
     [--export <dir>] [--yes-upload] [--accept-unknown-price]"
}
fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}

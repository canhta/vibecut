use crate::arguments::utf8;
use crate::exit_codes::CliError;
use crate::output::{Output, json_path};
use engine::{EditError, NfcText, Session, subtitles};
use project::WriterLease;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use time::Tick;

pub(crate) fn captions(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli captions import <project> <file.ass>".into())
    })?;
    let action = utf8(&action, "captions action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "import" => captions_import(rest),
        other => Err(CliError::Usage(format!(
            "unknown captions action: {other}; expected import"
        ))),
    }
}

fn captions_import(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut project_path: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut language = "en".to_owned();
    let mut start = Tick::ZERO;
    let mut track: Option<project::TrackId> = None;
    let mut name: Option<NfcText> = None;
    while let Some(arg) = args.next() {
        let flag = utf8(&arg, "captions flag")?;
        match flag {
            "--language" => {
                language = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--language requires a tag".into()))?,
                    "language",
                )?
                .to_owned();
            }
            "--start" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--start requires ticks".into()))?;
                let value = utf8(&raw, "start")?;
                start = Tick(
                    value
                        .parse()
                        .map_err(|_| CliError::Usage("--start requires integer ticks".into()))?,
                );
            }
            "--track" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--track requires an id".into()))?;
                let value = utf8(&raw, "track")?;
                let id = value
                    .parse()
                    .map_err(|_| CliError::Usage("--track requires a numeric id".into()))?;
                track = Some(project::TrackId(id));
            }
            "--name" => {
                name = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--name requires text".into()))?,
                        "name",
                    )?
                    .into(),
                );
            }
            "--help" | "-h" => {
                return Ok(Output {
                    silent: false,
                    exit: 0,
                    value: json!({"usage": captions_import_usage()}),
                    human: captions_import_usage().to_owned(),
                });
            }
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown captions flag: {other}")));
            }
            _ => {
                if project_path.is_none() {
                    project_path = Some(PathBuf::from(arg));
                } else if file.is_none() {
                    file = Some(PathBuf::from(arg));
                } else {
                    return Err(CliError::Usage(captions_import_usage().into()));
                }
            }
        }
    }
    let project_path =
        project_path.ok_or_else(|| CliError::Usage(captions_import_usage().into()))?;
    let file = file.ok_or_else(|| CliError::Usage(captions_import_usage().into()))?;
    if !file
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "ass" | "ssa"))
    {
        return Err(CliError::Usage(
            "captions import accepts .ass or .ssa files".into(),
        ));
    }
    let language = subtitles::parse_language(&language)?;

    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    session.restore_metadata(lease.metadata())?;
    let asset_id = session
        .project()
        .data()
        .assets
        .iter()
        .map(|asset| asset.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(EditError::Limit)?;
    let report = engine::prepare_ass_import(&engine::AssImportRequest {
        path: file,
        language,
        start,
        track,
        name,
        asset_id: media::AssetId(asset_id),
    })?;
    let before = session.project().clone();
    let outcome = session.apply(report.command)?;
    if outcome.changed && before != *session.project() {
        let _receipt = lease.save(session.project().as_ref())?;
        session.saved()?;
    }
    if outcome.changed {
        let mut metadata = session.metadata().clone();
        metadata.touch();
        lease.save_metadata(&metadata)?;
    }
    let created = serde_json::to_value(&outcome)?;
    let warnings = serde_json::to_value(&report.warnings)?;
    let value = json!({
        "project": json_path(&project_path)?,
        "cues": report.cues,
        "styles": report.styles,
        "skipped_cues": report.skipped_cues,
        "skipped_styles": report.skipped_styles,
        "warnings": warnings,
        "suppressed_warnings": report.suppressed_warnings,
        "created": created.get("created").cloned().unwrap_or(Value::Null),
        "placements": created.get("placements").cloned().unwrap_or(Value::Null),
    });
    let mut human = format!(
        "imported {} cues from {} styles ({} skipped cues, {} skipped styles)",
        report.cues, report.styles, report.skipped_cues, report.skipped_styles
    );
    for warning in &report.warnings {
        let _ = write!(
            human,
            "\nwarning line {}: {}",
            warning.line,
            warning.kind.label()
        );
    }
    if report.suppressed_warnings > 0 {
        let _ = write!(
            human,
            "\nsuppressed {} additional warnings",
            report.suppressed_warnings
        );
    }
    Ok(Output {
        silent: false,
        exit: 0,
        value,
        human,
    })
}

fn captions_import_usage() -> &'static str {
    "usage: vibecut-cli captions import <project> <file.ass> [--language <tag>]\n\
                         [--start <ticks>] [--track <id>] [--name <name>]"
}

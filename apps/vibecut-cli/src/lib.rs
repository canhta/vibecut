//! Headless CLI adapter: arguments, dispatch through engine, typed exit codes.
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{JobId, JobRunner, JobSnapshot, JobStage, JobState, RunnerConfig, SystemClock};
use engine::subtitles::{
    self, CatalogOperation, ConsentPrompt, Cost, LanguageTag, Progress, ProviderOption, Stage,
    SubtitleOutcome, SubtitleRequest, UploadRequest,
};
use engine::{ByteBudget, CreatedId, EditError, Session};
use project::atomic::Overwrite;
use project::{Project, ProjectId, WriterLease};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use time::{FrameRate, Tick, TimeRange};

const HELP: &str = "\
vibecut-cli - headless VibeCut control

Usage:
  vibecut-cli probe <file>
  vibecut-cli project new <path>
  vibecut-cli project info <path>
  vibecut-cli exec <project> <command_id> <json>
  vibecut-cli export <project> --out <mp4> [--aspect 9:16] [--allow-degraded] [--overwrite]
  vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]
                   [--srt <out>] [--burn <out.mp4>] [--aspect 9:16]
                   [--provider <id>] [--model <id>] [--translate-provider <id>]
                   [--translate-model <id>] [--yes-upload] [--overwrite]
  vibecut-cli providers [--operation transcribe|translate] [--language <tag>]
  vibecut-cli model list
  vibecut-cli model info <id>
  vibecut-cli model download <id> --yes-download
  vibecut-cli commands
  vibecut-cli --help
  vibecut-cli --version

Global:
  --json  print a JSON result on stdout; progress and errors go to stderr

Commands mutate projects only through the engine registry. Run
`vibecut-cli commands --json` for the generated command and parameter schema.";

/// One command result. Both forms are always built so `--json` never re-runs work.
pub struct Output {
    value: Value,
    human: String,
}
impl Output {
    pub fn write(&self, json_output: bool) -> Result<(), CliError> {
        let text = if json_output {
            serde_json::to_string_pretty(&self.value)?
        } else {
            self.human.clone()
        };
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{text}")?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Project(#[from] project::Error),
    #[error(transparent)]
    Engine(#[from] EditError),
    #[error(transparent)]
    Render(#[from] render::Error),
    #[error(transparent)]
    Media(#[from] media::MediaError),
    #[error(transparent)]
    Time(#[from] time::TimeError),
    #[error("{0}")]
    Job(Arc<EditError>),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Rejected(&'static str),
    #[error("{0}")]
    Unsupported(&'static str),
}
impl CliError {
    /// Stable machine code and process exit status. Documented in docs/product/cli.md.
    pub fn code(&self) -> (&'static str, u8) {
        match self {
            Self::Usage(_) | Self::Json(_) => ("usage", 2),
            Self::Project(project::Error::Busy) => ("busy", 3),
            Self::Project(project::Error::Conflict) => ("conflict", 3),
            Self::Project(project::Error::Io(_)) => ("io", 7),
            Self::Project(project::Error::Media(_)) => ("media", 6),
            Self::Project(_) => ("rejected", 5),
            Self::Engine(error) => edit_code(error),
            Self::Render(render::Error::Unavailable(_)) => ("unavailable", 4),
            Self::Render(_) => ("render", 6),
            Self::Media(_) => ("media", 6),
            Self::Time(_) => ("time", 5),
            Self::Job(error) => edit_code(error),
            Self::Io(_) => ("io", 7),
            Self::Rejected(_) => ("rejected", 5),
            Self::Unsupported(_) => ("unsupported", 5),
        }
    }
    pub fn exit_code(&self) -> u8 {
        self.code().1
    }
    /// Safe diagnostics: never project text or provider payloads.
    pub fn report(&self, json_output: bool) {
        let line = if json_output {
            let (code, exit) = self.code();
            let mut error = json!({
                "code": code,
                "message": self.to_string(),
                "exit_code": exit,
            });
            if let Some(list) = self.unavailable() {
                let unavailable: Vec<Value> = list.iter().map(unavailable_json).collect();
                if let Value::Object(fields) = &mut error {
                    let _ = fields.insert("unavailable".into(), Value::Array(unavailable));
                }
            }
            json!({"status": "error", "error": error}).to_string()
        } else {
            format!("error: {self}")
        };
        let _ = writeln!(io::stderr(), "{line}");
    }
    fn unavailable(&self) -> Option<&[render::Unavailable]> {
        match self {
            Self::Render(render::Error::Unavailable(list)) => Some(list),
            Self::Job(error) => match error.as_ref() {
                EditError::Render(render::Error::Unavailable(list)) => Some(list),
                _ => None,
            },
            _ => None,
        }
    }
}

fn edit_code(error: &EditError) -> (&'static str, u8) {
    match error {
        EditError::Render(render::Error::Unavailable(_)) => ("unavailable", 4),
        EditError::Render(_) => ("render", 6),
        EditError::Media(_) => ("media", 6),
        EditError::ModelMissing(_) | EditError::RunnerMissing | EditError::CredentialMissing(_) => {
            ("setup", 4)
        }
        EditError::Ai(_) => ("ai", 4),
        EditError::JobFailure(error) => edit_code(error),
        EditError::ConsentRequired | EditError::AutoLanguageUnsupported => ("rejected", 5),
        EditError::Project(project::Error::Busy) => ("busy", 3),
        EditError::Project(project::Error::Conflict) => ("conflict", 3),
        EditError::Project(project::Error::Io(_)) => ("io", 7),
        EditError::Project(project::Error::Media(_)) => ("media", 6),
        EditError::UnknownCommand | EditError::InvalidParams | EditError::Limit => ("usage", 2),
        EditError::Io(_) => ("io", 7),
        EditError::Cancelled
        | EditError::WorkerPanicked
        | EditError::JobPoisoned
        | EditError::QueueFull
        | EditError::BudgetFull
        | EditError::ArtifactTooLarge
        | EditError::NetworkTransient
        | EditError::StaleResult
        | EditError::CacheCorrupt => ("job", 7),
        _ => ("rejected", 5),
    }
}

fn unavailable_json(value: &render::Unavailable) -> Value {
    json!({
        "asset": value.asset.0,
        "item": value.item.0,
        "source_start": value.range.start().0,
        "source_end": value.range.end().0,
        "reason": reason_name(value.reason),
    })
}
fn reason_name(reason: render::UnavailableReason) -> &'static str {
    match reason {
        render::UnavailableReason::Missing => "missing",
        render::UnavailableReason::Changed => "changed",
        render::UnavailableReason::Metadata => "metadata",
        render::UnavailableReason::Video => "video",
        render::UnavailableReason::Audio => "audio",
    }
}

pub fn run(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(CliError::Usage(
            "missing subcommand; run `vibecut-cli --help`".into(),
        ));
    };
    let rest: Vec<OsString> = args.collect();
    let command = utf8(&command, "subcommand")?;
    match command {
        "--help" | "-h" | "help" => Ok(Output {
            value: json!({"usage": HELP}),
            human: HELP.to_owned(),
        }),
        "--version" | "-V" => Ok(Output {
            value: json!({"version": env!("CARGO_PKG_VERSION")}),
            human: format!("vibecut-cli {}", env!("CARGO_PKG_VERSION")),
        }),
        "probe" => probe(rest),
        "project" => project(rest),
        "exec" => exec(rest),
        "export" => export(rest),
        "subtitle" => subtitle(rest),
        "providers" => providers(rest),
        "model" => model(rest),
        "commands" => commands(rest),
        other => Err(CliError::Usage(format!("unknown subcommand: {other}"))),
    }
}

fn probe(args: Vec<OsString>) -> Result<Output, CliError> {
    let path = one_path(args, "usage: vibecut-cli probe <file>")?;
    let probe = media::probe(&path)?;
    let streams: Vec<Value> = probe.streams.iter().map(stream_json).collect();
    let value = json!({
        "format": probe.format,
        "duration": optional_tick_json(probe.duration),
        "bytes_read": probe.bytes_read,
        "streams": streams,
    });
    let mut human = format!("format: {}\n", probe.format);
    match probe.duration {
        Some(duration) => {
            let _ = writeln!(
                human,
                "duration: {:.3} s ({} ticks)",
                duration.seconds(),
                duration.0
            );
        }
        None => human.push_str("duration: unknown\n"),
    }
    for stream in &probe.streams {
        let _ = writeln!(human, "{}", stream_human(stream));
    }
    Ok(Output { value, human })
}

fn stream_json(stream: &media::StreamInfo) -> Value {
    json!({
        "index": stream.index,
        "kind": stream_kind(stream.kind),
        "codec": stream.codec,
        "time_base": {
            "numerator": stream.time_base.numerator(),
            "denominator": stream.time_base.denominator(),
        },
        "duration": optional_tick_json(stream.duration),
        "start": optional_tick_json(stream.start),
        "frame_rate": stream.frame_rate.map(|rate| json!({
            "num": rate.num(),
            "den": rate.den(),
            "fps": rate.as_f64(),
        })),
        "frame_count": stream.frame_count,
        "variable_frame_rate": stream.variable_frame_rate,
        "video": stream.video.as_ref().map(|video| json!({
            "width": video.width,
            "height": video.height,
            "color": video.color,
            "bit_depth": video.bit_depth,
        })),
        "audio": stream.audio.as_ref().map(|audio| json!({
            "sample_rate": audio.sample_rate,
            "channels": audio.channels,
            "initial_padding": audio.initial_padding,
            "trailing_padding": audio.trailing_padding,
        })),
    })
}

fn stream_human(stream: &media::StreamInfo) -> String {
    let mut line = format!(
        "stream {}: {} {}",
        stream.index,
        stream_kind(stream.kind),
        stream.codec
    );
    if let Some(video) = &stream.video {
        let _ = write!(line, " {}x{}", video.width, video.height);
    }
    if let Some(rate) = stream.frame_rate {
        let _ = write!(line, " {} fps", rate.label());
    }
    if let Some(audio) = &stream.audio {
        let _ = write!(line, " {} Hz {} ch", audio.sample_rate, audio.channels);
    }
    line
}

fn stream_kind(kind: media::StreamKind) -> &'static str {
    match kind {
        media::StreamKind::Video => "video",
        media::StreamKind::Audio => "audio",
        media::StreamKind::Other => "other",
    }
}

fn tick_json(tick: Tick) -> Value {
    json!({"ticks": tick.0, "seconds": tick.seconds()})
}
fn optional_tick_json(tick: Option<Tick>) -> Value {
    tick.map_or(Value::Null, tick_json)
}

fn project(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args
        .next()
        .ok_or_else(|| CliError::Usage("usage: vibecut-cli project new|info <path>".into()))?;
    let action = utf8(&action, "project action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "new" => project_new(one_path(rest, "usage: vibecut-cli project new <path>")?),
        "info" => project_info(one_path(rest, "usage: vibecut-cli project info <path>")?),
        other => Err(CliError::Usage(format!(
            "unknown project action: {other}; expected new or info"
        ))),
    }
}

fn project_new(path: PathBuf) -> Result<Output, CliError> {
    let mut lease = WriterLease::acquire(&path)?;
    match lease.load() {
        Ok(_) => {
            return Err(CliError::Usage(format!(
                "project already exists: {}",
                path.display()
            )));
        }
        Err(project::Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(CliError::Project(error)),
    }
    let project = Project::empty(ProjectId(0), "Untitled".into())?;
    let receipt = lease.save(&project)?;
    let value = json!({
        "path": json_path(receipt.path())?,
        "content_hash": hex(&receipt.content_hash().0),
    });
    Ok(Output {
        value,
        human: format!("created {}", receipt.path().display()),
    })
}

fn project_info(path: PathBuf) -> Result<Output, CliError> {
    let loaded = project::load(&path)?;
    let data = loaded.project.data();
    let duration = loaded.project.duration();
    let tracks = data.timeline.tracks.len();
    let items = data
        .timeline
        .tracks
        .iter()
        .map(|t| t.items.len())
        .sum::<usize>();
    let fps = data.canvas.fps;
    let value = json!({
        "path": json_path(&path)?,
        "id": data.id.0,
        "name": data.name.as_str(),
        "canvas": {
            "width": data.canvas.width,
            "height": data.canvas.height,
            "fps": fps_json(fps),
            "background": data.canvas.background,
        },
        "duration": {"ticks": duration.0, "seconds": duration.seconds()},
        "assets": data.assets.len(),
        "tracks": tracks,
        "items": items,
        "bookmarks": data.bookmarks.len(),
        "schema_version": loaded.schema_version,
    });
    let human = format!(
        "path: {}\nname: {}\ncanvas: {}x{} @ {} fps\nduration: {:.3} s ({} ticks)\nassets: {}, tracks: {}, items: {}, bookmarks: {}\nschema: {}",
        path.display(),
        data.name.as_str(),
        data.canvas.width,
        data.canvas.height,
        fps.label(),
        duration.seconds(),
        duration.0,
        data.assets.len(),
        tracks,
        items,
        data.bookmarks.len(),
        loaded.schema_version,
    );
    Ok(Output { value, human })
}

fn fps_json(rate: FrameRate) -> Value {
    json!({"num": rate.num(), "den": rate.den(), "fps": rate.as_f64()})
}

fn exec(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(exec_usage)?;
    let command_id = args.next().ok_or_else(exec_usage)?;
    let params = args.next().ok_or_else(exec_usage)?;
    if args.next().is_some() {
        return Err(exec_usage());
    }
    let command_id = utf8(&command_id, "command id")?;
    let params = utf8(&params, "command JSON")?;
    let params: Value = serde_json::from_str(params).map_err(CliError::Json)?;

    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 1)?;
    let outcome = session.execute(command_id, params)?;
    if outcome.changed {
        let _receipt = lease.save(session.project().as_ref())?;
        session.saved()?;
    }
    let value = serde_json::to_value(&outcome)?;
    Ok(Output {
        value,
        human: outcome_human(&outcome),
    })
}

fn exec_usage() -> CliError {
    CliError::Usage("usage: vibecut-cli exec <project> <command_id> <json>".into())
}

fn outcome_human(outcome: &engine::Outcome) -> String {
    let mut human = if outcome.changed {
        "changed\n".to_owned()
    } else {
        "no change\n".to_owned()
    };
    for created in &outcome.created {
        let _ = writeln!(human, "created {}", created_human(created));
    }
    human.trim_end().to_owned()
}

fn created_human(created: &CreatedId) -> String {
    match created {
        CreatedId::Track(id) => format!("track {}", id.0),
        CreatedId::Item(id) => format!("item {}", id.0),
        CreatedId::Bookmark(id) => format!("bookmark {}", id.0),
    }
}

fn export(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(export_usage)?;
    let mut output = None;
    let mut aspect = None;
    let mut allow_degraded = false;
    let mut overwrite = false;
    while let Some(arg) = args.next() {
        match utf8(&arg, "export flag")? {
            "--out" => {
                output = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--out requires an output path".into())
                })?));
            }
            "--aspect" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--aspect requires 9:16 or 16:9".into()))?;
                aspect = Some(match utf8(&value, "aspect")? {
                    "9:16" => Aspect::Portrait,
                    "16:9" => Aspect::Landscape,
                    _ => {
                        return Err(CliError::Usage("--aspect must be 9:16 or 16:9".into()));
                    }
                });
            }
            "--allow-degraded" => allow_degraded = true,
            "--overwrite" => overwrite = true,
            other => {
                return Err(CliError::Usage(format!("unknown export flag: {other}")));
            }
        }
    }
    let output = output.ok_or_else(export_usage)?;
    export_project(project_path, output, aspect, allow_degraded, overwrite)
}

fn export_usage() -> CliError {
    CliError::Usage(
        "usage: vibecut-cli export <project> --out <mp4> [--aspect 9:16] [--allow-degraded] [--overwrite]"
            .into(),
    )
}

#[derive(Clone, Copy)]
enum Aspect {
    Portrait,
    Landscape,
}

/// No CLI preferences owner yet; 2 GiB admits one 1080p export's working buffers.
const CLI_MEMORY_LIMIT: u64 = 2 << 30;

fn export_project(
    project_path: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
) -> Result<Output, CliError> {
    let loaded = project::load(&project_path)?;
    let project = loaded.project;
    if same_target(&project_path, &output)? {
        return Err(CliError::Usage(
            "export output must not replace the project file".into(),
        ));
    }
    let duration = project.duration();
    if duration <= Tick::ZERO {
        return Err(CliError::Rejected("project timeline is empty"));
    }
    let range = TimeRange::new(Tick::ZERO, duration)?;
    let settings = export_settings(&project, aspect, allow_degraded, overwrite);
    let project_bytes = project::encode(&project, false)?;
    let source = media::ContentHash::from_bytes(&project_bytes).0;
    let clip_range =
        media::ContentHash::from_bytes(format!("{}:{}", range.start().0, range.end().0).as_bytes())
            .0;
    let settings_key = format!(
        "{}x{}@{}/{}:{}:{}:{}",
        settings.render.width,
        settings.render.height,
        settings.fps.num(),
        settings.fps.den(),
        settings.bit_rate,
        settings.quality,
        settings.include_audio,
    );
    // Export has no persistent artifact kind; this key only identifies the job.
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source,
        provider: "vibecut-cli",
        model: "render",
        version: "1",
        settings: settings_key.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range,
        translation: [0; 32],
    })?;

    let budget = engine::ByteBudget::new(CLI_MEMORY_LIMIT)?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(20),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?;
    let (id, _cancel) = engine::submit_export(
        &mut runner,
        key,
        Arc::new(project),
        range,
        settings,
        output.clone(),
    )?;
    wait_for_job(&mut runner, id)?;
    runner.shutdown();
    let value = json!({
        "path": json_path(&output)?,
        "status": "completed",
    });
    Ok(Output {
        value,
        human: format!("exported {}", output.display()),
    })
}

fn export_settings(
    project: &Project,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
) -> render::ExportSettings {
    let mut settings = render::ExportSettings::from_project(project);
    settings.render = match aspect {
        Some(Aspect::Portrait) => render::RenderSettings::portrait(project),
        Some(Aspect::Landscape) => {
            let mut render = render::RenderSettings::from_project(project);
            render.width = 1920;
            render.height = 1080;
            render
        }
        None => render::RenderSettings::from_project(project),
    };
    settings.allow_degraded = allow_degraded;
    settings.overwrite = if overwrite {
        Overwrite::Replace
    } else {
        Overwrite::Refuse
    };
    settings
}

fn wait_for_job(runner: &mut JobRunner, id: JobId) -> Result<(), CliError> {
    let mut last = (JobStage::Queued, u32::MAX);
    loop {
        runner.tick()?;
        for event in runner.drain_events() {
            if event.id != id {
                continue;
            }
            let percent = progress_percent(event.progress);
            if last != (event.stage, percent) {
                last = (event.stage, percent);
                let _ = writeln!(
                    io::stderr(),
                    "progress {} {}%",
                    stage_name(event.stage),
                    percent
                );
            }
        }
        let snapshot = runner
            .snapshot(id)
            .ok_or(CliError::Rejected("export job disappeared"))?;
        match snapshot.state {
            JobState::Completed => return Ok(()),
            JobState::Failed => {
                return Err(match snapshot.error {
                    Some(error) => CliError::Job(error),
                    None => CliError::Rejected("export failed without diagnostics"),
                });
            }
            JobState::Cancelled => {
                return Err(CliError::Job(Arc::new(EditError::Cancelled)));
            }
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn progress_percent(progress: f32) -> u32 {
    if progress.is_finite() {
        (progress.clamp(0.0, 1.0) * 100.0).round() as u32
    } else {
        0
    }
}

fn stage_name(stage: JobStage) -> &'static str {
    match stage {
        JobStage::Queued => "queued",
        JobStage::Starting => "starting",
        JobStage::Transcribe => "transcribe",
        JobStage::Align => "align",
        JobStage::Translate => "translate",
        JobStage::SuggestClips => "suggest_clips",
        JobStage::Assemble => "assemble",
        JobStage::Process => "process",
        JobStage::Admit => "admit",
        JobStage::Render => "render",
        JobStage::Mix => "mix",
        JobStage::Encode => "encode",
        JobStage::Finalize => "finalize",
        JobStage::Download => "download",
        JobStage::Verify => "verify",
        JobStage::Publish => "publish",
        JobStage::Backoff => "backoff",
        JobStage::Complete => "complete",
    }
}

fn subtitle(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut input: Option<PathBuf> = None;
    let mut from = "auto".to_owned();
    let mut to: Option<String> = None;
    let mut bilingual = false;
    let mut srt: Option<PathBuf> = None;
    let mut burn: Option<PathBuf> = None;
    let mut portrait = None;
    let mut provider = "whisper.cpp".to_owned();
    let mut model: Option<String> = None;
    let mut translate_provider = "openai".to_owned();
    let mut translate_model: Option<String> = None;
    let mut yes_upload = false;
    let mut overwrite = false;
    while let Some(arg) = args.next() {
        let flag = utf8(&arg, "subtitle flag")?;
        match flag {
            "--from" => {
                from = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--from requires a language".into()))?,
                    "source language",
                )?
                .to_owned();
            }
            "--to" => {
                to = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--to requires a language".into()))?,
                        "target language",
                    )?
                    .to_owned(),
                );
            }
            "--bilingual" => bilingual = true,
            "--srt" => {
                srt = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--srt requires an output path".into())
                })?));
            }
            "--burn" => {
                burn = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--burn requires an output path".into())
                })?));
            }
            "--aspect" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--aspect requires 9:16 or 16:9".into()))?;
                let value = utf8(&raw, "aspect")?;
                portrait = Some(match value {
                    "9:16" => true,
                    "16:9" => false,
                    _ => return Err(CliError::Usage("--aspect must be 9:16 or 16:9".into())),
                });
            }
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "model",
                    )?
                    .to_owned(),
                );
            }
            "--translate-provider" => {
                translate_provider = utf8(
                    &args.next().ok_or_else(|| {
                        CliError::Usage("--translate-provider requires an id".into())
                    })?,
                    "translation provider",
                )?
                .to_owned();
            }
            "--translate-model" => {
                translate_model = Some(
                    utf8(
                        &args.next().ok_or_else(|| {
                            CliError::Usage("--translate-model requires an id".into())
                        })?,
                        "translation model",
                    )?
                    .to_owned(),
                );
            }
            "--yes-upload" => yes_upload = true,
            "--overwrite" => overwrite = true,
            "--help" | "-h" => {
                return Ok(Output {
                    value: json!({"usage": subtitle_usage()}),
                    human: subtitle_usage().to_owned(),
                });
            }
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown subtitle flag: {other}")));
            }
            _ => {
                if input.is_some() {
                    return Err(CliError::Usage(
                        "subtitle takes one input media path".into(),
                    ));
                }
                input = Some(PathBuf::from(arg));
            }
        }
    }
    let input = input.ok_or_else(subtitle_usage_error)?;
    let to = to.ok_or_else(subtitle_usage_error)?;
    if srt.is_none() && burn.is_none() {
        return Err(CliError::Usage(
            "subtitle requires --srt and/or --burn".into(),
        ));
    }
    if portrait.is_some() && burn.is_none() {
        return Err(CliError::Usage("--aspect requires --burn".into()));
    }
    let source_language = match from.as_str() {
        "auto" => None,
        value => {
            if !["vi", "en", "zh"].contains(&value) {
                return Err(CliError::Usage("--from must be vi, en, zh or auto".into()));
            }
            Some(subtitles::parse_language(value)?)
        }
    };
    let target_language = subtitles::normalize_target_tag(&subtitles::parse_language(&to)?)?;
    if !["vi", "en", "zh-Hans", "zh-Hant"].contains(&target_language.as_str()) {
        return Err(CliError::Usage(
            "--to must be vi, en, zh, zh-Hans or zh-Hant".into(),
        ));
    }
    if bilingual
        && source_language
            .as_ref()
            .is_some_and(|source| source == &target_language)
    {
        return Err(CliError::Usage(
            "--bilingual requires --from and --to to differ".into(),
        ));
    }
    let transcription = subtitles::choose_transcription(&provider, model.as_deref())?;
    let credentials = subtitles::Credentials::new(
        std::env::var("OPENAI_API_KEY").ok(),
        std::env::var("ELEVENLABS_API_KEY").ok(),
        std::env::var("ANTHROPIC_API_KEY").ok(),
    )?;
    let mut runtime = subtitles::Runtime::default();
    if let Ok(value) = std::env::var("VIBECUT_ASR_THREADS") {
        let threads = value.parse::<u8>().map_err(|_| {
            CliError::Usage("VIBECUT_ASR_THREADS must be an integer from 1 to 32".into())
        })?;
        if !(1..=32).contains(&threads) {
            return Err(CliError::Usage(
                "VIBECUT_ASR_THREADS must be an integer from 1 to 32".into(),
            ));
        }
        runtime.threads = threads;
    }
    if std::env::var("VIBECUT_ASR_GPU").is_ok_and(|value| value == "0") {
        runtime.use_gpu = false;
    }
    runtime.gipformer_runner = local_asr_runner();
    let budget = ByteBudget::new(CLI_MEMORY_LIMIT)?;
    let root = subtitles::default_cache_root()?;
    let (mut cache, _diagnostics) = ArtifactCache::open(&root, budget.clone(), 128)?;
    let cancellation = subtitles::Cancellation::new();
    let mut source_language = source_language;
    let translation = match &source_language {
        Some(source) if translation_required(source, &target_language, bilingual) => Some(
            subtitles::choose_translation(&translate_provider, translate_model.as_deref())?,
        ),
        _ => None,
    };
    let mut providers = subtitles::Providers::open(
        transcription,
        translation,
        &mut cache,
        &credentials,
        &runtime,
    )?;
    if source_language.is_none() {
        let detected = providers.detect_language_for(&input, &cancellation)?;
        if bilingual && detected == target_language {
            return Err(CliError::Usage(
                "--bilingual requires the detected source and --to to differ".into(),
            ));
        }
        if translation_required(&detected, &target_language, bilingual) {
            providers = providers.with_translation(
                subtitles::choose_translation(&translate_provider, translate_model.as_deref())?,
                &credentials,
            )?;
        }
        source_language = Some(detected);
    }
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 8,
            retained: 8,
            network_attempts: 3,
            backoff: Duration::from_millis(250),
            progress_interval: Duration::from_millis(50),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?;
    let request = SubtitleRequest {
        input: input.clone(),
        source_language,
        target_language,
        bilingual,
        srt,
        burn,
        portrait,
        overwrite,
    };
    let progress = CliProgress {
        last: std::sync::Mutex::new((JobStage::Queued, u32::MAX)),
    };
    let consent = CliConsent { yes_upload };
    let outcome = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &consent,
        &progress,
        &cancellation,
    )?;
    runner.shutdown();
    let value = outcome_json(&outcome, &input)?;
    Ok(Output {
        value,
        human: outcome_human_subtitle(&outcome),
    })
}

fn translation_required(source: &LanguageTag, target: &LanguageTag, bilingual: bool) -> bool {
    bilingual || source != target
}

fn subtitle_usage() -> &'static str {
    "usage: vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]\n\
                     [--srt <out>] [--burn <out.mp4>] [--aspect 9:16]\n\
                     [--provider <id>] [--model <id>] [--translate-provider <id>]\n\
                     [--translate-model <id>] [--yes-upload] [--overwrite]"
}
fn subtitle_usage_error() -> CliError {
    CliError::Usage(subtitle_usage().into())
}

fn local_asr_runner() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("VIBECUT_LOCAL_ASR") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let path = parent.join("vibecut-local-asr");
        if path.is_file() {
            return Some(path);
        }
    }
    let path = Path::new("target")
        .join("release")
        .join("vibecut-local-asr");
    path.is_file().then_some(path)
}

struct CliProgress {
    last: std::sync::Mutex<(JobStage, u32)>,
}
impl Progress for CliProgress {
    fn stage(&self, stage: Stage) {
        let _ = writeln!(io::stderr(), "stage {}", subtitle_stage_name(stage));
    }
    fn job(&self, snapshot: &JobSnapshot) {
        let percent = progress_percent(snapshot.progress);
        if let Ok(mut last) = self.last.lock()
            && *last != (snapshot.stage, percent)
        {
            *last = (snapshot.stage, percent);
            let _ = writeln!(
                io::stderr(),
                "progress {} {}%",
                stage_name(snapshot.stage),
                percent
            );
        }
    }
}
fn subtitle_stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Extract => "extract",
        Stage::Transcribe => "transcribe",
        Stage::Align => "align",
        Stage::Assemble => "assemble",
        Stage::Segment => "segment",
        Stage::Translate => "translate",
        Stage::ExportSrt => "export_srt",
        Stage::ExportBurn => "export_burn",
    }
}

struct CliConsent {
    yes_upload: bool,
}
impl ConsentPrompt for CliConsent {
    fn approve(&self, request: &UploadRequest) -> Result<(), EditError> {
        let operation = match request.operation {
            CatalogOperation::Transcribe => "transcribe",
            CatalogOperation::Translate => "translate",
        };
        let _ = writeln!(
            io::stderr(),
            "upload consent: {operation} {} {} ({})",
            request.provider,
            request.model,
            request.languages
        );
        if request.audio_seconds > 0.0 {
            let _ = writeln!(io::stderr(), "audio: {:.3} s", request.audio_seconds);
        }
        let _ = writeln!(io::stderr(), "payload: {} bytes", request.upload_bytes);
        let _ = writeln!(io::stderr(), "cost: {}", cost_label(&request.cost));
        for (label, value) in &request.terms {
            let _ = writeln!(io::stderr(), "{label}: {value}");
        }
        if self.yes_upload {
            Ok(())
        } else {
            let _ = writeln!(
                io::stderr(),
                "consent: refused; pass --yes-upload to authorize this exact scope"
            );
            Err(EditError::ConsentRequired)
        }
    }
}

fn cost_label(cost: &Cost) -> String {
    match cost {
        Cost::Free => "free".into(),
        Cost::Unknown => "unknown".into(),
        Cost::UpperBound { currency, micros } => {
            format!("{currency} {:.6}", *micros as f64 / 1_000_000.0)
        }
    }
}
fn cost_json(cost: &Cost) -> Value {
    match cost {
        Cost::Free => json!({"kind": "free"}),
        Cost::Unknown => json!({"kind": "unknown"}),
        Cost::UpperBound { currency, micros } => {
            json!({"kind": "upper_bound", "currency": currency, "micros": micros})
        }
    }
}
fn outcome_json(outcome: &SubtitleOutcome, input: &Path) -> Result<Value, CliError> {
    let path = |value: &Option<PathBuf>| -> Result<Value, CliError> {
        match value {
            Some(value) => Ok(Value::String(json_path(value)?)),
            None => Ok(Value::Null),
        }
    };
    Ok(json!({
        "status": "completed",
        "input": json_path(input)?,
        "source_language": outcome.source_language.as_str(),
        "target_language": outcome.target_language.as_str(),
        "provider": {
            "id": outcome.transcription.id(),
            "model": outcome.transcription.model(),
        },
        "translation_provider": outcome.translation.as_ref().map(|value| json!({
            "id": value.id(),
            "model": value.model(),
        })),
        "segments": outcome.segments,
        "cues": outcome.cues,
        "checkpoints": outcome.checkpoints,
        "srt": path(&outcome.srt)?,
        "burn": path(&outcome.burn)?,
        "transcription_cost": cost_json(&outcome.transcription_cost),
        "translation_cost": cost_json(&outcome.translation_cost),
        "elapsed_seconds": outcome.elapsed.as_secs_f64(),
    }))
}
fn outcome_human_subtitle(outcome: &SubtitleOutcome) -> String {
    let mut human = format!(
        "source: {}\ntarget: {}\nprovider: {} ({})\n",
        outcome.source_language.as_str(),
        outcome.target_language.as_str(),
        outcome.transcription.id(),
        outcome.transcription.model()
    );
    match &outcome.translation {
        Some(value) => {
            let _ = writeln!(human, "translation: {} ({})", value.id(), value.model());
        }
        None => human.push_str("translation: none\n"),
    }
    let _ = writeln!(
        human,
        "segments: {}\ncues: {}\ncheckpoints reused: {}",
        outcome.segments, outcome.cues, outcome.checkpoints
    );
    if let Some(path) = &outcome.srt {
        let _ = writeln!(human, "srt: {}", path.display());
    }
    if let Some(path) = &outcome.burn {
        let _ = writeln!(human, "burn: {}", path.display());
    }
    let _ = writeln!(
        human,
        "cost: transcribe {}; translate {}\nelapsed: {:.3} s",
        cost_label(&outcome.transcription_cost),
        cost_label(&outcome.translation_cost),
        outcome.elapsed.as_secs_f64()
    );
    human.trim_end().to_owned()
}

fn providers(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut operation = CatalogOperation::Transcribe;
    let mut language = None;
    while let Some(arg) = args.next() {
        match utf8(&arg, "providers flag")? {
            "--operation" => {
                let raw = args.next().ok_or_else(|| {
                    CliError::Usage("--operation requires transcribe or translate".into())
                })?;
                let value = utf8(&raw, "operation")?;
                operation = match value {
                    "transcribe" => CatalogOperation::Transcribe,
                    "translate" => CatalogOperation::Translate,
                    _ => {
                        return Err(CliError::Usage(
                            "--operation must be transcribe or translate".into(),
                        ));
                    }
                };
            }
            "--language" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--language requires a tag".into()))?;
                let value = utf8(&raw, "language")?;
                language = Some(subtitles::parse_language(value)?);
            }
            other => {
                return Err(CliError::Usage(format!("unknown providers flag: {other}")));
            }
        }
    }
    let options = subtitles::provider_options_for(operation, language.as_ref());
    let value: Vec<Value> = options.iter().map(provider_option_json).collect();
    let human = options
        .iter()
        .map(|option| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                option.id,
                option.label,
                option.models.join(","),
                option.languages.join(","),
                if option.local { "local" } else { "remote" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Output {
        value: json!({"providers": value}),
        human,
    })
}
fn provider_option_json(option: &ProviderOption) -> Value {
    json!({
        "id": option.id,
        "label": option.label,
        "operation": match option.operation {
            CatalogOperation::Transcribe => "transcribe",
            CatalogOperation::Translate => "translate",
        },
        "location": if option.local { "local" } else { "remote" },
        "models": option.models,
        "languages": option.languages,
        "target_languages": option.target_languages,
        "native_timing": option.native_timing,
    })
}

fn model(args: Vec<OsString>) -> Result<Output, CliError> {
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
            Ok(Output {
                value: json!({"models": value}),
                human,
            })
        }
        "info" => {
            let id = one_utf8(rest, "usage: vibecut-cli model info <id>")?;
            let model = subtitles::model_info(&id)?;
            Ok(Output {
                value: model_json(&model),
                human: model_human(&model),
            })
        }
        "download" => model_download(rest),
        other => Err(CliError::Usage(format!(
            "unknown model action: {other}; expected list, info or download"
        ))),
    }
}
fn model_download(args: Vec<OsString>) -> Result<Output, CliError> {
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
    let budget = ByteBudget::new(CLI_MEMORY_LIMIT)?;
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
    Ok(Output {
        value: json!({"model": info.id, "status": "installed"}),
        human: format!("installed {}", info.id),
    })
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
fn one_utf8(args: Vec<OsString>, usage: &str) -> Result<String, CliError> {
    let mut args = args.into_iter();
    let value = args
        .next()
        .ok_or_else(|| CliError::Usage(usage.to_owned()))?;
    if args.next().is_some() {
        return Err(CliError::Usage(usage.to_owned()));
    }
    Ok(utf8(&value, "argument")?.to_owned())
}

fn commands(args: Vec<OsString>) -> Result<Output, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: vibecut-cli commands".into()));
    }
    let registry = engine::registry();
    let value = serde_json::to_value(registry)?;
    let human = registry
        .iter()
        .map(|spec| {
            let fields = spec
                .params_schema
                .iter()
                .map(|field| format!("{}: {}", field.name, field.wire_type))
                .collect::<Vec<_>>()
                .join(", ");
            if fields.is_empty() {
                format!("{} - {}", spec.id, spec.label)
            } else {
                format!("{} - {} ({fields})", spec.id, spec.label)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Output { value, human })
}

fn one_path(args: Vec<OsString>, usage: &str) -> Result<PathBuf, CliError> {
    let mut args = args.into_iter();
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Usage(usage.to_owned()))?;
    if args.next().is_some() {
        return Err(CliError::Usage(usage.to_owned()));
    }
    Ok(path)
}

fn utf8<'a>(value: &'a OsString, what: &str) -> Result<&'a str, CliError> {
    value
        .to_str()
        .ok_or_else(|| CliError::Usage(format!("{what} must be valid UTF-8")))
}

fn json_path(path: &Path) -> Result<String, CliError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(CliError::Unsupported(
            "non-UTF-8 path cannot be represented in JSON",
        ))
}

fn same_target(left: &Path, right: &Path) -> Result<bool, CliError> {
    Ok(resolve_target(left)? == resolve_target(right)?)
}

fn resolve_target(path: &Path) -> Result<PathBuf, CliError> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or(CliError::Usage("path has no file name".into()))?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(std::fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let high = DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
        let low = DIGITS
            .get(usize::from(byte & 0x0f))
            .copied()
            .unwrap_or(b'0');
        output.push(char::from(high));
        output.push(char::from(low));
    }
    output
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn portrait_and_landscape_override_only_output_geometry() {
        let project = Project::empty(ProjectId(0), "".into()).unwrap();
        let portrait = export_settings(&project, Some(Aspect::Portrait), false, false);
        assert_eq!(
            (portrait.render.width, portrait.render.height),
            (1080, 1920)
        );
        let landscape = export_settings(&project, Some(Aspect::Landscape), false, false);
        assert_eq!(
            (landscape.render.width, landscape.render.height),
            (1920, 1080)
        );
        assert_eq!(landscape.fps, project.data().canvas.fps);
        assert!(!landscape.allow_degraded);
        assert!(matches!(landscape.overwrite, Overwrite::Refuse));
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(CliError::Usage("".into()).exit_code(), 2);
        assert_eq!(CliError::Project(project::Error::Busy).exit_code(), 3);
        assert_eq!(CliError::Project(project::Error::Conflict).exit_code(), 3);
        assert_eq!(
            CliError::Render(render::Error::Unavailable(Vec::new())).exit_code(),
            4
        );
        assert_eq!(CliError::Rejected("").exit_code(), 5);
        assert_eq!(CliError::Render(render::Error::Cancelled).exit_code(), 6);
        assert_eq!(
            CliError::Media(media::MediaError::Invalid("")).exit_code(),
            6
        );
        assert_eq!(CliError::Io(io::Error::other("")).exit_code(), 7);
    }

    #[test]
    fn translation_is_needed_only_for_different_languages_or_bilingual() {
        let vi = LanguageTag::parse("vi").unwrap();
        let en = LanguageTag::parse("en").unwrap();
        assert!(!translation_required(&vi, &vi, false));
        assert!(translation_required(&vi, &en, false));
        assert!(translation_required(&vi, &vi, true));
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}

//! Headless CLI adapter: arguments, dispatch through engine, typed exit codes.
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{JobRunner, JobStage, JobState, RunnerConfig, SystemClock};
use engine::{CreatedId, EditError, Session};
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
vibecut - headless VibeCut control

Usage:
  vibecut probe <file>
  vibecut project new <path>
  vibecut project info <path>
  vibecut exec <project> <command_id> <json>
  vibecut export <project> --out <mp4> [--aspect 9:16] [--allow-degraded] [--overwrite]
  vibecut commands
  vibecut --help
  vibecut --version

Global:
  --json  print a JSON result on stdout; progress and errors go to stderr

Commands mutate projects only through the engine registry. Run
`vibecut commands --json` for the generated command and parameter schema.";

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
            "missing subcommand; run `vibecut --help`".into(),
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
            human: format!("vibecut {}", env!("CARGO_PKG_VERSION")),
        }),
        "probe" => probe(rest),
        "project" => project(rest),
        "exec" => exec(rest),
        "export" => export(rest),
        "commands" => commands(rest),
        other => Err(CliError::Usage(format!("unknown subcommand: {other}"))),
    }
}

fn probe(args: Vec<OsString>) -> Result<Output, CliError> {
    let path = one_path(args, "usage: vibecut probe <file>")?;
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
        .ok_or_else(|| CliError::Usage("usage: vibecut project new|info <path>".into()))?;
    let action = utf8(&action, "project action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "new" => project_new(one_path(rest, "usage: vibecut project new <path>")?),
        "info" => project_info(one_path(rest, "usage: vibecut project info <path>")?),
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
    CliError::Usage("usage: vibecut exec <project> <command_id> <json>".into())
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
        "usage: vibecut export <project> --out <mp4> [--aspect 9:16] [--allow-degraded] [--overwrite]"
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
    wait_for_export(&mut runner, id)?;
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

fn wait_for_export(runner: &mut JobRunner, id: engine::jobs::JobId) -> Result<(), CliError> {
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

fn commands(args: Vec<OsString>) -> Result<Output, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: vibecut commands".into()));
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
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}

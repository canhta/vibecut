//! Headless CLI adapter: arguments, dispatch through engine, typed exit codes.
use engine::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use engine::clip_suggest;
use engine::control::{
    self, Caller, ControlResult, ErrorCode, Policy, Request, Response, ResponseValue,
};
use engine::jobs::{JobId, JobRunner, JobSnapshot, JobStage, JobState, RunnerConfig, SystemClock};
use engine::subtitles::{
    self, CatalogOperation, ConsentPrompt, Cost, LanguageTag, Progress, ProviderOption, Stage,
    SubtitleOutcome, SubtitleRequest, UploadRequest,
};
use engine::{
    ByteBudget, ClipCandidate, ClipDropReason, ClipExportRequest, ClipRequest, ClipRole,
    ClipSettings, ClipSource, ClipSourceSegment, ClipSuggestionOutcome, Command, CreatedId,
    EditError, NfcText, Session, SnapSettings,
};
use project::atomic::Overwrite;
use project::{Project, ProjectId, WriterLease};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write as _};
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
  vibecut-cli control <project> [--allow-overwrite] [--no-save]
  vibecut-cli export <project> --out <mp4> [--aspect 9:16] [--allow-degraded] [--overwrite]
  vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]
                   [--srt <out>] [--burn <out.mp4>] [--aspect 9:16]
                   [--provider <id>] [--model <id>] [--translate-provider <id>]
                   [--translate-model <id>] [--yes-upload] [--overwrite]
  vibecut-cli clips <in> [--from vi|en|zh|auto] [--count N]
                    [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s]
                    [--provider heuristic|openai|anthropic] [--model <id>]
                    [--asr-provider <id>] [--asr-model <id>] [--keywords a,b]
                    [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual]
                     [--aspect 9:16] [--translate-provider <id>] [--overwrite]
                     [--allow-degraded]]
                    [--yes-upload]
  vibecut-cli providers [--operation transcribe|translate|clips] [--language <tag>]
  vibecut-cli model list
  vibecut-cli model info <id>
  vibecut-cli model download <id> --yes-download
  vibecut-cli commands
  vibecut-cli schema
  vibecut-cli --help
  vibecut-cli --version

Global:
  --json  print a JSON result on stdout; progress and errors go to stderr

Commands mutate projects only through the engine registry. Run
`vibecut-cli commands --json` for the generated command and parameter schema.";

/// One command result. Both forms are always built so `--json` never re-runs work.
/// A streamed command writes its own protocol and returns a silent result with
/// the process status of its final frame.
pub struct Output {
    value: Value,
    human: String,
    silent: bool,
    exit: u8,
}
impl Output {
    pub fn write(&self, json_output: bool) -> Result<(), CliError> {
        if self.silent {
            return Ok(());
        }
        let text = if json_output {
            serde_json::to_string_pretty(&self.value)?
        } else {
            self.human.clone()
        };
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{text}")?;
        Ok(())
    }
    pub fn exit_code(&self) -> u8 {
        self.exit
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

fn cli_error_code(error: &CliError) -> ErrorCode {
    match error {
        CliError::Engine(error) => ErrorCode::from_error(error),
        CliError::Job(error) => ErrorCode::from_error(error),
        CliError::Project(_) | CliError::Io(_) => ErrorCode::Failed,
        CliError::Usage(_) | CliError::Json(_) => ErrorCode::InvalidParams,
        _ => ErrorCode::Failed,
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
            silent: false,
            exit: 0,
            value: json!({"usage": HELP}),
            human: HELP.to_owned(),
        }),
        "--version" | "-V" => Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"version": env!("CARGO_PKG_VERSION")}),
            human: format!("vibecut-cli {}", env!("CARGO_PKG_VERSION")),
        }),
        "probe" => probe(rest),
        "project" => project(rest),
        "exec" => exec(rest),
        "control" => control(rest),
        "export" => export(rest),
        "subtitle" => subtitle(rest),
        "clips" => clips(rest),
        "providers" => providers(rest),
        "model" => model(rest),
        "commands" => commands(rest),
        "schema" => schema(rest),
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
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
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
    let session = Session::create(
        engine::Command::CreateProject {
            id: ProjectId(0),
            name: "Untitled".into(),
        },
        1,
    )?;
    let receipt = lease.save(session.project())?;
    lease.save_metadata(session.metadata())?;
    let value = json!({
        "path": json_path(receipt.path())?,
        "content_hash": hex(&receipt.content_hash().0),
    });
    Ok(Output {
        silent: false,
        exit: 0,
        value,
        human: format!("created {}", receipt.path().display()),
    })
}

fn project_info(path: PathBuf) -> Result<Output, CliError> {
    let lease = WriterLease::acquire(&path)?;
    let loaded = lease.load()?;
    let metadata = lease.metadata();
    let data = loaded.project.data();
    let duration = loaded.project.duration();
    let tracks = data.scene().timeline.track_count();
    let items = data
        .scene()
        .timeline
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
        "bookmarks": data.scene().bookmarks.len(),
        "schema_version": loaded.schema_version,
        "metadata": metadata,
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
        data.scene().bookmarks.len(),
        loaded.schema_version,
    );
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
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
    session.restore_metadata(lease.metadata())?;
    if command_id == "project.save" {
        if params != json!({}) {
            return Err(EditError::InvalidParams.into());
        }
        let _ = lease.save(session.project())?;
        lease.save_metadata(session.metadata())?;
        return Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"changed":false,"created":[]}),
            human: "saved".into(),
        });
    }
    let before = session.project().clone();
    let outcome = session.execute(command_id, params)?;
    if outcome.changed && before != *session.project() {
        let _receipt = lease.save(session.project().as_ref())?;
        session.saved()?;
    }
    if outcome.changed {
        let mut metadata = session.metadata().clone();
        metadata.touch();
        lease.save_metadata(&metadata)?;
    }
    let value = serde_json::to_value(&outcome)?;
    Ok(Output {
        silent: false,
        exit: 0,
        value,
        human: outcome_human(&outcome),
    })
}

fn exec_usage() -> CliError {
    CliError::Usage("usage: vibecut-cli exec <project> <command_id> <json>".into())
}

/// Trusted admission owned by this composition root; never deserialized from
/// client JSON. The CLI grants overwrite only when the flag was passed.
struct ControlPolicy {
    allow_overwrite: bool,
}
impl Policy for ControlPolicy {
    fn authorize(&self, command: &Command, _: &Project) -> engine::Result<()> {
        let overwrite = match command {
            Command::Export { overwrite, .. }
            | Command::ExportSubtitles { overwrite, .. }
            | Command::BurnSubtitles { overwrite, .. } => Some(*overwrite),
            _ => None,
        };
        if overwrite == Some(true) && !self.allow_overwrite {
            return Err(EditError::Permission);
        }
        Ok(())
    }
}

fn control_usage() -> CliError {
    CliError::Usage("usage: vibecut-cli control <project> [--allow-overwrite] [--no-save]".into())
}

fn control(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(project) = args.next() else {
        return Err(control_usage());
    };
    if project
        .to_str()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        let usage = "usage: vibecut-cli control <project> [--allow-overwrite] [--no-save]\n\
                     Reads newline-delimited control requests on stdin and writes one JSON\n\
                     response per line. The session saves changed project/metadata on EOF\n\
                     unless --no-save is set; --allow-overwrite grants FilePublish overwrite.";
        return Ok(Output {
            value: json!({"usage": usage}),
            human: usage.to_owned(),
            silent: false,
            exit: 0,
        });
    }
    let project_path = PathBuf::from(project);
    let mut allow_overwrite = false;
    let mut save = true;
    for arg in args {
        match utf8(&arg, "control flag")? {
            "--allow-overwrite" => allow_overwrite = true,
            "--no-save" => save = false,
            other => return Err(CliError::Usage(format!("unknown control flag: {other}"))),
        }
    }

    // One process is one engine Session. The lease is held from load to the
    // final save, so no other writer can interleave during the stream.
    let mut lease = WriterLease::acquire(&project_path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, 100)?;
    session.restore_metadata(lease.metadata())?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 32,
            retained: 32,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(50),
        },
        ByteBudget::new(CLI_MEMORY_LIMIT)?,
        Arc::new(SystemClock::default()),
    )?;
    let policy = ControlPolicy { allow_overwrite };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut project_dirty = false;
    let mut metadata_dirty = false;
    let mut last_exit = 0u8;

    loop {
        runner.tick()?;
        session.observe_jobs(&runner);
        let Some(bytes) = read_control_frame(&mut input, control::MAX_FRAME_BYTES)? else {
            break;
        };
        if bytes.is_empty() {
            continue;
        }
        let value = match control::parse_frame(&bytes) {
            Ok(value) => value,
            Err(error) => {
                let code = ErrorCode::from_error(&error);
                write_control_error(&mut out, None, code, frame_error_message(&error))?;
                last_exit = control_exit(code);
                continue;
            }
        };
        let request = match build_control_request(&session, value) {
            Ok(request) => request,
            Err(error) => {
                let code = ErrorCode::from_error(&error);
                write_control_error(
                    &mut out,
                    None,
                    code,
                    "control request has an invalid envelope",
                )?;
                last_exit = control_exit(code);
                continue;
            }
        };
        let request_id = request.request_id.clone();
        let command_id = request.command.clone();
        let response = if command_id == "project.save" {
            match save_control_request(&mut session, &mut lease, &request) {
                Ok(response) => {
                    project_dirty = false;
                    metadata_dirty = false;
                    response
                }
                Err(error) => {
                    let code = cli_error_code(&error);
                    write_control_error(&mut out, Some(&request_id), code, "project save failed")?;
                    last_exit = control_exit(code);
                    continue;
                }
            }
        } else {
            let before = session.project().clone();
            let caller = Caller {
                principal: "vibecut-cli",
                external: true,
                policy: &policy,
            };
            match session.control_request(request, caller, &mut runner) {
                Ok(response) => {
                    if matches!(
                        &response.value,
                        ResponseValue::Result(ControlResult::Edit { outcome }) if outcome.changed
                    ) {
                        metadata_dirty = true;
                        if command_id != "project.set_view_state" {
                            project_dirty = true;
                        }
                    }
                    if before != *session.project() {
                        project_dirty = true;
                    }
                    response
                }
                Err(error) => {
                    let code = ErrorCode::from_error(&error);
                    write_control_error(
                        &mut out,
                        Some(&request_id),
                        code,
                        "control request refused before admission",
                    )?;
                    last_exit = control_exit(code);
                    continue;
                }
            }
        };
        last_exit = control_response_exit(&response.value);
        write_control_response(&mut out, &response)?;
    }

    runner.shutdown();
    if save && (project_dirty || metadata_dirty) {
        save_control_state(&mut session, &mut lease, project_dirty, metadata_dirty)?;
    }
    Ok(Output {
        value: Value::Null,
        human: String::new(),
        silent: true,
        exit: last_exit,
    })
}

fn build_control_request(session: &Session, mut value: Value) -> Result<Request, EditError> {
    let Value::Object(fields) = &mut value else {
        return Err(EditError::InvalidParams);
    };
    fields
        .entry("api_version")
        .or_insert_with(|| json!(control::API_VERSION));
    fields
        .entry("registry_hash")
        .or_insert_with(|| json!(control::registry_hash()));
    fields
        .entry("session_id")
        .or_insert_with(|| json!(session.session_id()));
    let request: Request = serde_json::from_value(value).map_err(|_| EditError::InvalidParams)?;
    Ok(request)
}

fn save_control_request(
    session: &mut Session,
    lease: &mut WriterLease,
    request: &Request,
) -> Result<Response, CliError> {
    let before = session.revision();
    let code = if request.api_version != control::API_VERSION {
        Some(ErrorCode::SchemaMismatch)
    } else if request.session_id != session.session_id() {
        Some(ErrorCode::SessionGone)
    } else if request.registry_hash.as_deref() != Some(control::registry_hash()) {
        Some(ErrorCode::SchemaMismatch)
    } else if request.params != json!({}) || request.idempotency_key.is_none() {
        Some(ErrorCode::InvalidParams)
    } else if request
        .idempotency_key
        .as_ref()
        .is_some_and(|key| key.is_empty() || key.len() > control::MAX_KEY_BYTES)
    {
        Some(ErrorCode::Limit)
    } else if request.base_rev.as_ref() != Some(&session.revision()) {
        Some(ErrorCode::StaleRevision)
    } else {
        None
    };
    if let Some(code) = code {
        return Ok(Response {
            api_version: control::API_VERSION,
            registry_hash: control::registry_hash().into(),
            session_id: session.session_id().into(),
            request_id: request.request_id.clone(),
            rev_before: before,
            rev_after: session.revision(),
            value: ResponseValue::Error(code),
        });
    }
    save_control_state(session, lease, true, true)?;
    Ok(Response {
        api_version: control::API_VERSION,
        registry_hash: control::registry_hash().into(),
        session_id: session.session_id().into(),
        request_id: request.request_id.clone(),
        rev_before: before,
        rev_after: session.revision(),
        value: ResponseValue::Result(ControlResult::Edit {
            outcome: engine::Outcome {
                changed: false,
                created: Vec::new(),
                placements: Vec::new(),
            },
        }),
    })
}

fn save_control_state(
    session: &mut Session,
    lease: &mut WriterLease,
    project_dirty: bool,
    metadata_dirty: bool,
) -> Result<(), CliError> {
    if project_dirty {
        let _receipt = lease.save(session.project())?;
    }
    if metadata_dirty {
        let mut metadata = session.metadata().clone();
        metadata.touch();
        lease.save_metadata(&metadata)?;
        session.restore_metadata(metadata)?;
    }
    if project_dirty || metadata_dirty {
        session.saved()?;
    }
    Ok(())
}

fn control_response_exit(value: &ResponseValue) -> u8 {
    match value {
        ResponseValue::Result(_) => 0,
        ResponseValue::Error(code) => control_exit(*code),
    }
}

fn control_exit(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::UnknownCommand
        | ErrorCode::InvalidParams
        | ErrorCode::Limit
        | ErrorCode::SchemaMismatch => 2,
        ErrorCode::StaleRevision => 3,
        ErrorCode::Permission
        | ErrorCode::IdempotencyConflict
        | ErrorCode::ReceiptCapacity
        | ErrorCode::TransactionActive
        | ErrorCode::Refused => 5,
        ErrorCode::SessionGone
        | ErrorCode::ResyncRequired
        | ErrorCode::JobNotRetained
        | ErrorCode::Cancelled
        | ErrorCode::QueueFull
        | ErrorCode::Failed => 7,
    }
}

fn error_code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnknownCommand => "unknown_command",
        ErrorCode::InvalidParams => "invalid_params",
        ErrorCode::Limit => "limit",
        ErrorCode::SchemaMismatch => "schema_mismatch",
        ErrorCode::SessionGone => "session_gone",
        ErrorCode::StaleRevision => "stale_revision",
        ErrorCode::Permission => "permission",
        ErrorCode::IdempotencyConflict => "idempotency_conflict",
        ErrorCode::ReceiptCapacity => "receipt_capacity",
        ErrorCode::ResyncRequired => "resync_required",
        ErrorCode::JobNotRetained => "job_not_retained",
        ErrorCode::Cancelled => "cancelled",
        ErrorCode::QueueFull => "queue_full",
        ErrorCode::TransactionActive => "transaction_active",
        ErrorCode::Refused => "refused",
        ErrorCode::Failed => "failed",
    }
}

fn frame_error_message(error: &EditError) -> &'static str {
    match error {
        EditError::Limit => "control request exceeds the frame limit",
        _ => "control request is not valid control JSON",
    }
}

fn write_control_response(out: &mut impl io::Write, response: &Response) -> Result<(), CliError> {
    let mut value = serde_json::to_value(response)?;
    if let Value::Object(fields) = &mut value {
        fields.insert(
            "exit_code".into(),
            json!(control_response_exit(&response.value)),
        );
    }
    writeln!(out, "{value}")?;
    out.flush()?;
    Ok(())
}

fn write_control_error(
    out: &mut impl io::Write,
    request_id: Option<&str>,
    code: ErrorCode,
    message: &str,
) -> Result<(), CliError> {
    let exit = control_exit(code);
    let mut value = json!({
        "status": "error",
        "exit_code": exit,
        "error": {"code": error_code_name(code), "message": message, "exit_code": exit},
    });
    if let (Some(request_id), Value::Object(fields)) = (request_id, &mut value) {
        fields.insert("request_id".into(), json!(request_id));
    }
    writeln!(out, "{value}")?;
    out.flush()?;
    Ok(())
}

/// Read one bounded newline-framed request, draining an oversized line so the
/// next frame still starts at a line boundary.
fn read_control_frame(input: &mut impl BufRead, max: usize) -> Result<Option<Vec<u8>>, CliError> {
    let mut line = Vec::new();
    loop {
        let (take, done) = {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                break;
            }
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (buffer.len(), false),
            }
        };
        if line.len().saturating_add(take) > max.saturating_add(2) {
            input.consume(take);
            if !done {
                drain_control_line(input)?;
            }
            return Err(EditError::Limit.into());
        }
        let buffer = input.fill_buf()?;
        line.extend(buffer.iter().take(take).copied());
        input.consume(take);
        if done {
            break;
        }
    }
    while line
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        line.pop();
    }
    Ok(Some(line))
}

fn drain_control_line(input: &mut impl BufRead) -> Result<(), CliError> {
    loop {
        let (take, done) = {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                return Ok(());
            }
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (buffer.len(), false),
            }
        };
        input.consume(take);
        if done {
            return Ok(());
        }
    }
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
        CreatedId::Scene(id) => format!("scene {}", id.0),
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
    export_project_with(
        project_path,
        output,
        aspect,
        allow_degraded,
        overwrite,
        engine::submit_export,
    )
}

fn export_project_with(
    project_path: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
    submit: impl FnOnce(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, engine::jobs::CancelToken), EditError>,
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
    let (id, _cancel) = submit(
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
        silent: false,
        exit: 0,
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
        JobStage::Preparing => "preparing",
        JobStage::Analyzing => "analyzing",
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
                    silent: false,
                    exit: 0,
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
    let runtime = cli_runtime()?;
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
    let mut runner = cli_runner(budget)?;
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
        silent: false,
        exit: 0,
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

fn cli_runtime() -> Result<subtitles::Runtime, CliError> {
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
    Ok(runtime)
}

fn cli_runner(budget: ByteBudget) -> Result<JobRunner, CliError> {
    Ok(JobRunner::new(
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
    )?)
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
        Stage::SuggestClips => "suggest_clips",
        Stage::Snap => "snap",
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
            CatalogOperation::SuggestClips => "suggest_clips",
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

fn clips(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut input: Option<PathBuf> = None;
    let mut from = "auto".to_owned();
    let mut count = 5usize;
    let mut min = 20.0f64;
    let mut max = 60.0f64;
    let mut min_segment = 2.0f64;
    let mut max_segments = 4usize;
    let mut overlap = 0.5f64;
    let mut keywords: Vec<String> = Vec::new();
    let mut provider = "heuristic".to_owned();
    let mut model: Option<String> = None;
    let mut asr_provider = "whisper.cpp".to_owned();
    let mut asr_model: Option<String> = None;
    let mut translate_provider = "openai".to_owned();
    let mut translate_model: Option<String> = None;
    let mut select: Option<Vec<usize>> = None;
    let mut export: Option<PathBuf> = None;
    let mut subs: Option<String> = None;
    let mut bilingual = false;
    let mut aspect: Option<String> = None;
    let mut yes_upload = false;
    let mut overwrite = false;
    let mut allow_degraded = false;
    while let Some(arg) = args.next() {
        let flag = utf8(&arg, "clips flag")?;
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
            "--count" => count = number_arg(&mut args, "--count")?,
            "--min" => min = seconds_arg(&mut args, "--min")?,
            "--max" => max = seconds_arg(&mut args, "--max")?,
            "--min-segment" => min_segment = seconds_arg(&mut args, "--min-segment")?,
            "--max-segments" => max_segments = number_arg(&mut args, "--max-segments")?,
            "--overlap" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--overlap requires a ratio".into()))?;
                overlap = utf8(&value, "overlap")?
                    .parse()
                    .map_err(|_| CliError::Usage("--overlap must be a number in 0..=1".into()))?;
            }
            "--keywords" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--keywords requires a list".into()))?;
                keywords = utf8(&value, "keywords")?
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "suggestion provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "suggestion model",
                    )?
                    .to_owned(),
                );
            }
            "--asr-provider" => {
                asr_provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--asr-provider requires an id".into()))?,
                    "ASR provider",
                )?
                .to_owned();
            }
            "--asr-model" => {
                asr_model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--asr-model requires an id".into()))?,
                        "ASR model",
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
            "--select" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--select requires indices".into()))?;
                select = Some(parse_selection(utf8(&value, "selection")?)?);
            }
            "--export" => {
                export = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--export requires an output directory".into())
                })?));
            }
            "--subs" => {
                subs = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--subs requires a language".into()))?,
                        "subtitle language",
                    )?
                    .to_owned(),
                );
            }
            "--bilingual" => bilingual = true,
            "--aspect" => {
                aspect = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--aspect requires 9:16".into()))?,
                        "aspect",
                    )?
                    .to_owned(),
                );
            }
            "--yes-upload" => yes_upload = true,
            "--overwrite" => overwrite = true,
            "--allow-degraded" => allow_degraded = true,
            "--help" | "-h" => {
                return Ok(Output {
                    silent: false,
                    exit: 0,
                    value: json!({"usage": clips_usage()}),
                    human: clips_usage().to_owned(),
                });
            }
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown clips flag: {other}")));
            }
            _ => {
                if input.is_some() {
                    return Err(CliError::Usage("clips takes one input media path".into()));
                }
                input = Some(PathBuf::from(arg));
            }
        }
    }
    let input = input.ok_or_else(clips_usage_error)?;
    if !(1..=32).contains(&count) || !(1..=32).contains(&max_segments) {
        return Err(CliError::Usage(
            "--count and --max-segments must be 1..=32".into(),
        ));
    }
    if !min.is_finite() || !max.is_finite() || !min_segment.is_finite() || min <= 0.0 {
        return Err(CliError::Usage(
            "--min, --max and --min-segment must be positive".into(),
        ));
    }
    if max < min || min_segment > max {
        return Err(CliError::Usage(
            "--max must be at least --min and --min-segment".into(),
        ));
    }
    if !overlap.is_finite() || !(0.0..=1.0).contains(&overlap) {
        return Err(CliError::Usage(
            "--overlap must be a number in 0..=1".into(),
        ));
    }
    if keywords.len() > 64 || keywords.iter().any(|value| value.len() > 256) {
        return Err(CliError::Usage(
            "at most 64 keywords of at most 256 bytes each".into(),
        ));
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
    let target_language = match &subs {
        None => None,
        Some(value) => {
            if !["vi", "en", "zh"].contains(&value.as_str()) {
                return Err(CliError::Usage("--subs must be vi, en or zh".into()));
            }
            Some(subtitles::normalize_target_tag(
                &subtitles::parse_language(value)?,
            )?)
        }
    };
    if export.is_none() && (subs.is_some() || bilingual || aspect.is_some() || select.is_some()) {
        return Err(CliError::Usage(
            "--subs, --bilingual, --aspect and --select require --export".into(),
        ));
    }
    if export.is_some() && subs.is_none() {
        return Err(CliError::Usage("--export requires --subs".into()));
    }
    if let Some(aspect) = &aspect
        && aspect != "9:16"
    {
        if aspect == "16:9" {
            return Err(CliError::Unsupported("clip export is portrait 9:16 only"));
        }
        return Err(CliError::Usage("--aspect must be 9:16".into()));
    }
    let suggestion = clip_suggest::choose_suggestion(&provider, model.as_deref())?;
    let transcription = subtitles::choose_transcription(&asr_provider, asr_model.as_deref())?;
    let credentials = subtitles::Credentials::new(
        std::env::var("OPENAI_API_KEY").ok(),
        std::env::var("ELEVENLABS_API_KEY").ok(),
        std::env::var("ANTHROPIC_API_KEY").ok(),
    )?;
    clip_suggest::check_suggestion_credentials(&suggestion, &credentials)?;
    let runtime = cli_runtime()?;
    let budget = ByteBudget::new(CLI_MEMORY_LIMIT)?;
    let root = subtitles::default_cache_root()?;
    let (mut cache, _diagnostics) = ArtifactCache::open(&root, budget.clone(), 128)?;
    let cancellation = subtitles::Cancellation::new();
    let mut providers =
        subtitles::Providers::open(transcription, None, &mut cache, &credentials, &runtime)?;
    let source_language = match source_language {
        Some(language) => language,
        None => providers.detect_language_for(&input, &cancellation)?,
    };
    if let Some(target) = &target_language {
        if bilingual && &source_language == target {
            return Err(CliError::Usage(
                "--bilingual requires the source and --subs to differ".into(),
            ));
        }
        if &source_language != target || bilingual {
            providers = providers.with_translation(
                subtitles::choose_translation(&translate_provider, translate_model.as_deref())?,
                &credentials,
            )?;
        }
    }
    let settings = ClipSettings {
        count,
        min_duration: seconds_ticks(min)?,
        max_duration: seconds_ticks(max)?,
        max_segments,
        min_segment_duration: seconds_ticks(min_segment)?,
        overlap_tolerance: overlap,
        language: source_language.clone(),
        keywords: keywords
            .iter()
            .map(|value| NfcText::from(value.as_str()))
            .collect(),
    };
    let request = ClipRequest {
        input: input.clone(),
        source_language: Some(source_language.clone()),
        settings: settings.clone(),
        snap: SnapSettings {
            frame_rate: None,
            tolerance: seconds_ticks(1.0)?,
            silence_rms: 0.01,
        },
        provider: suggestion,
    };
    let mut runner = cli_runner(budget)?;
    let progress = CliProgress {
        last: std::sync::Mutex::new((JobStage::Queued, u32::MAX)),
    };
    let consent = CliConsent { yes_upload };
    let outcome = clip_suggest::suggest(
        &request,
        &credentials,
        &providers,
        &mut runner,
        &mut cache,
        &consent,
        &progress,
        &cancellation,
    )?;
    let mut selected = match select {
        Some(values) => values,
        None => (0..outcome.clips.len()).collect(),
    };
    for index in &selected {
        if *index >= outcome.clips.len() {
            return Err(CliError::Usage(format!(
                "--select index {} is outside 1..={}",
                index + 1,
                outcome.clips.len()
            )));
        }
    }
    selected.sort_unstable();
    let languages = match &target_language {
        Some(target) if bilingual => vec![source_language.clone(), target.clone()],
        Some(target) => vec![target.clone()],
        None => Vec::new(),
    };
    let mut exported = None;
    if let Some(directory) = export {
        if selected.is_empty() {
            return Err(CliError::Rejected("no clip candidates to export"));
        }
        let result = clip_suggest::export(
            &outcome,
            ClipExportRequest {
                directory,
                select: selected,
                languages: languages.clone(),
                formats: Vec::new(),
                overwrite,
                allow_degraded,
                max_output_bytes: 64 << 30,
            },
            &providers,
            &mut runner,
            &consent,
            &progress,
            &cancellation,
        )?;
        exported = Some(result);
    }
    runner.shutdown();
    let value = clips_json(&outcome, &input, &settings, exported.as_ref(), &languages)?;
    Ok(Output {
        silent: false,
        exit: 0,
        value,
        human: clips_human(&outcome, exported.as_ref()),
    })
}

fn clips_usage() -> &'static str {
    "usage: vibecut-cli clips <in> [--from vi|en|zh|auto] [--count N]\n\
                    [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s]\n\
                    [--provider heuristic|openai|anthropic] [--model <id>]\n\
                    [--asr-provider <id>] [--asr-model <id>] [--keywords a,b]\n\
                    [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual]\n\
                     [--aspect 9:16] [--translate-provider <id>] [--overwrite]\n\
                     [--allow-degraded]] [--yes-upload]"
}
fn clips_usage_error() -> CliError {
    CliError::Usage(clips_usage().into())
}
fn number_arg(args: &mut std::vec::IntoIter<OsString>, flag: &str) -> Result<usize, CliError> {
    let raw = args
        .next()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a number")))?;
    let value = utf8(&raw, flag)?;
    value
        .parse()
        .map_err(|_| CliError::Usage(format!("{flag} requires a nonnegative integer")))
}
fn seconds_arg(args: &mut std::vec::IntoIter<OsString>, flag: &str) -> Result<f64, CliError> {
    let raw = args
        .next()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a duration such as 20s")))?;
    let value = utf8(&raw, flag)?;
    let number = value.strip_suffix('s').unwrap_or(value);
    let parsed: f64 = number
        .parse()
        .map_err(|_| CliError::Usage(format!("{flag} must be seconds such as 20s")))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(CliError::Usage(format!("{flag} must be positive")));
    }
    Ok(parsed)
}
fn seconds_ticks(value: f64) -> Result<Tick, CliError> {
    Ok(Tick::from_seconds_f64(value, time::Rounding::Nearest)?)
}
fn parse_selection(value: &str) -> Result<Vec<usize>, CliError> {
    let mut result: Vec<usize> = Vec::new();
    for part in value.split(',') {
        let number: usize = part.trim().parse().map_err(|_| {
            CliError::Usage("--select takes comma-separated 1-based indices".into())
        })?;
        if number == 0 {
            return Err(CliError::Usage("--select indices start at 1".into()));
        }
        let index = number - 1;
        if result.contains(&index) {
            return Err(CliError::Usage("--select indices must be distinct".into()));
        }
        result.push(index);
    }
    if result.is_empty() || result.len() > 1000 {
        return Err(CliError::Usage(
            "--select takes 1..=1000 comma-separated indices".into(),
        ));
    }
    Ok(result)
}

fn clips_json(
    outcome: &ClipSuggestionOutcome,
    input: &Path,
    settings: &ClipSettings,
    exported: Option<&engine::ClipExportOutcome>,
    languages: &[LanguageTag],
) -> Result<Value, CliError> {
    let candidates: Vec<Value> = outcome
        .clips
        .iter()
        .enumerate()
        .map(|(index, candidate)| candidate_json(index, candidate))
        .collect();
    let rejected: Vec<Value> = outcome
        .rejected
        .iter()
        .map(|clip| {
            json!({
                "title": clip.title.as_str(),
                "reason": boundary_failure_name(clip.reason),
            })
        })
        .collect();
    let dropped: Vec<Value> = outcome
        .dropped
        .iter()
        .map(|clip| {
            json!({
                "title": clip.proposal.title.as_str(),
                "reason": drop_reason_name(clip.reason),
            })
        })
        .collect();
    let exports: Vec<Value> = match exported {
        Some(result) => result
            .exports
            .iter()
            .map(|clip| {
                Ok(json!({
                    "candidate": clip.candidate + 1,
                    "video": json_path(&clip.paths.video)?,
                    "subtitles": clip.paths.subtitles.iter().map(|subtitle| {
                        Ok(json!({
                            "format": format_name(subtitle.format),
                            "path": json_path(&subtitle.path)?,
                        }))
                    }).collect::<Result<Vec<_>, CliError>>()?,
                }))
            })
            .collect::<Result<Vec<_>, CliError>>()?,
        None => Vec::new(),
    };
    Ok(json!({
        "status": "completed",
        "input": json_path(input)?,
        "source_language": outcome.source_language.as_str(),
        "transcription": {
            "provider": outcome.transcription_provider,
            "model": outcome.transcription_model,
        },
        "suggestion": {
            "provider": outcome.provider,
            "model": outcome.model,
        },
        "settings": {
            "count": settings.count,
            "min_duration": tick_json(settings.min_duration),
            "max_duration": tick_json(settings.max_duration),
            "max_segments": settings.max_segments,
            "min_segment_duration": tick_json(settings.min_segment_duration),
            "overlap_tolerance": settings.overlap_tolerance,
            "keywords": settings.keywords.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
        },
        "candidates": candidates,
        "rejected": rejected,
        "dropped": dropped,
        "transcript_segments": outcome.transcript_segments,
        "cues": outcome.cues,
        "checkpoints": outcome.checkpoints,
        "cost": {
            "transcription": cost_json(&outcome.transcription_cost),
            "suggestion": cost_json(&outcome.suggestion_cost),
            "translation": exported.map_or(json!({"kind": "free"}), |value| cost_json(&value.translation_cost)),
        },
        "subtitle_languages": languages.iter().map(LanguageTag::as_str).collect::<Vec<_>>(),
        "exports": exports,
        "elapsed_seconds": outcome.elapsed.as_secs_f64(),
    }))
}

fn candidate_json(index: usize, candidate: &ClipCandidate) -> Value {
    json!({
        "index": index + 1,
        "title": candidate.proposal.title.as_str(),
        "hook_text": candidate.proposal.hook_text.as_str(),
        "reason": candidate.proposal.reason.as_str(),
        "score": candidate.proposal.score,
        "source": clip_source_name(candidate.source),
        "duration": tick_json(candidate.duration),
        "segments": candidate.segments.iter().map(segment_json).collect::<Vec<_>>(),
        "adjustments": candidate.adjustments.iter().map(|pair| json!({
            "start": adjustment_json(&pair.start),
            "end": adjustment_json(&pair.end),
        })).collect::<Vec<_>>(),
        "pieces": candidate.proposal.segments.iter().map(|piece| json!({
            "start_segment_id": piece.start_segment_id.as_str(),
            "end_segment_id": piece.end_segment_id.as_str(),
            "role": clip_role_name(piece.role),
        })).collect::<Vec<_>>(),
    })
}
fn segment_json(segment: &ClipSourceSegment) -> Value {
    json!({
        "start": tick_json(segment.range.start()),
        "end": tick_json(segment.range.end()),
        "role": clip_role_name(segment.role),
        "source_order": segment.source_order,
    })
}
fn adjustment_json(value: &engine::clip_boundaries::BoundaryAdjustment) -> Value {
    json!({
        "original": tick_json(value.original),
        "adjusted": tick_json(value.adjusted),
        "delta": tick_json(value.delta),
        "kind": boundary_kind_name(value.kind),
    })
}
fn clip_source_name(source: ClipSource) -> &'static str {
    match source {
        ClipSource::Llm => "llm",
        ClipSource::Heuristic => "heuristic",
    }
}
fn clip_role_name(role: ClipRole) -> &'static str {
    match role {
        ClipRole::Hook => "hook",
        ClipRole::Context => "context",
        ClipRole::Payoff => "payoff",
    }
}
fn boundary_kind_name(kind: engine::clip_boundaries::BoundaryKind) -> &'static str {
    match kind {
        engine::clip_boundaries::BoundaryKind::SentenceSilence => "sentence_silence",
        engine::clip_boundaries::BoundaryKind::Sentence => "sentence",
        engine::clip_boundaries::BoundaryKind::Silence => "silence",
        engine::clip_boundaries::BoundaryKind::WordOrSegment => "word_or_segment",
        engine::clip_boundaries::BoundaryKind::WordFade => "word_fade",
    }
}
fn boundary_failure_name(reason: engine::clip_boundaries::BoundaryFailure) -> &'static str {
    match reason {
        engine::clip_boundaries::BoundaryFailure::NoSafeFrame => "no_safe_frame",
        engine::clip_boundaries::BoundaryFailure::SegmentDuration => "segment_duration",
        engine::clip_boundaries::BoundaryFailure::TotalDuration => "total_duration",
        engine::clip_boundaries::BoundaryFailure::SegmentOverlap => "segment_overlap",
        engine::clip_boundaries::BoundaryFailure::Candidate => "candidate",
    }
}
fn drop_reason_name(reason: ClipDropReason) -> &'static str {
    match reason {
        ClipDropReason::Duration => "duration",
        ClipDropReason::SegmentDuration => "segment_duration",
        ClipDropReason::SegmentCount => "segment_count",
        ClipDropReason::UnknownId => "unknown_id",
        ClipDropReason::ReversedRange => "reversed_range",
        ClipDropReason::HookOrder => "hook_order",
        ClipDropReason::SegmentOverlap => "segment_overlap",
        ClipDropReason::Duplicate => "duplicate",
        ClipDropReason::Overlap => "overlap",
        ClipDropReason::CountLimit => "count_limit",
    }
}
fn format_name(format: engine::Format) -> &'static str {
    match format {
        engine::Format::Srt => "srt",
        engine::Format::WebVtt => "web_vtt",
    }
}

fn clips_human(
    outcome: &ClipSuggestionOutcome,
    exported: Option<&engine::ClipExportOutcome>,
) -> String {
    let mut human = format!(
        "source: {}\nprovider: {} ({})\ntranscription: {} ({})\n",
        outcome.source_language.as_str(),
        outcome.provider,
        outcome.model,
        outcome.transcription_provider,
        outcome.transcription_model
    );
    let _ = writeln!(
        human,
        "candidates: {} ({} rejected, {} dropped)\ncheckpoints reused: {}",
        outcome.clips.len(),
        outcome.rejected.len(),
        outcome.dropped.len(),
        outcome.checkpoints
    );
    for (index, candidate) in outcome.clips.iter().enumerate() {
        let _ = writeln!(
            human,
            "{}. {:.3} s, {} segment(s), score {:.3}: {}",
            index + 1,
            candidate.duration.seconds(),
            candidate.segments.len(),
            candidate.proposal.score,
            candidate.proposal.title.as_str()
        );
        for segment in &candidate.segments {
            let _ = writeln!(
                human,
                "   {:.3}-{:.3} s {}",
                segment.range.start().seconds(),
                segment.range.end().seconds(),
                clip_role_name(segment.role)
            );
        }
    }
    for clip in &outcome.rejected {
        let _ = writeln!(
            human,
            "rejected ({}): {}",
            boundary_failure_name(clip.reason),
            clip.title.as_str()
        );
    }
    if let Some(result) = exported {
        for clip in &result.exports {
            let _ = writeln!(
                human,
                "exported {}: {}",
                clip.candidate + 1,
                clip.paths.video.display()
            );
            for subtitle in &clip.paths.subtitles {
                let _ = writeln!(
                    human,
                    "  {}: {}",
                    format_name(subtitle.format),
                    subtitle.path.display()
                );
            }
        }
    }
    let _ = writeln!(
        human,
        "cost: transcribe {}; suggest {}; translate {}",
        cost_label(&outcome.transcription_cost),
        cost_label(&outcome.suggestion_cost),
        cost_label(&exported.map_or(Cost::Free, |value| value.translation_cost.clone()))
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
                    "clips" => CatalogOperation::SuggestClips,
                    _ => {
                        return Err(CliError::Usage(
                            "--operation must be transcribe, translate or clips".into(),
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
    if operation == CatalogOperation::SuggestClips {
        let options = clip_suggest::suggestion_options();
        let value: Vec<Value> = options
            .iter()
            .map(|option| {
                json!({
                    "id": option.id,
                    "label": option.label,
                    "operation": "suggest_clips",
                    "location": if option.local { "local" } else { "remote" },
                    "models": option.models,
                    "policy_url": option.policy_url,
                })
            })
            .collect();
        let human = options
            .iter()
            .map(|option| {
                format!(
                    "{}\t{}\t{}\t{}",
                    option.id,
                    option.label,
                    option.models.join(","),
                    if option.local { "local" } else { "remote" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"providers": value}),
            human,
        });
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
        silent: false,
        exit: 0,
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
            CatalogOperation::SuggestClips => "suggest_clips",
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
                silent: false,
                exit: 0,
                value: json!({"models": value}),
                human,
            })
        }
        "info" => {
            let id = one_utf8(rest, "usage: vibecut-cli model info <id>")?;
            let model = subtitles::model_info(&id)?;
            Ok(Output {
                silent: false,
                exit: 0,
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
        silent: false,
        exit: 0,
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
    let value = json!({
        "api_version": control::API_VERSION,
        "registry_hash": control::registry_hash(),
        "limits": control::LIMITS,
        "entries": registry,
    });
    let mut human = format!(
        "API version: {}\nRegistry digest: {}\n",
        control::API_VERSION,
        control::registry_hash()
    );
    human.push_str(
        &registry
            .iter()
            .map(|spec| {
                let fields = spec
                    .params_schema
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                    .map(|fields| fields.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                if fields.is_empty() {
                    format!("{} - {}", spec.id, spec.label)
                } else {
                    format!("{} - {} ({fields})", spec.id, spec.label)
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
}

fn schema(args: Vec<OsString>) -> Result<Output, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: vibecut-cli schema".into()));
    }
    let value = json!({
        "api_version": control::API_VERSION,
        "registry_hash": control::registry_hash(),
        "contract_schema": control::contract_schema(),
    });
    let human = format!(
        "API version: {}\nRegistry digest: {}\nRequest, response and event JSON Schema 2020-12 envelopes.",
        control::API_VERSION,
        control::registry_hash()
    );
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
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

#[cfg(test)]
mod export_tests;

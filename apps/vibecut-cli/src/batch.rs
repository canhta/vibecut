//! Bounded, validated headless batch render over the engine export job owner.
//!
//! A job file names projects and destinations only. This adapter parses and
//! admits the request, loads each project once as an immutable snapshot, and
//! submits the existing `engine::submit_export` jobs in file order. Render owns
//! pixels, staging and the publication fence; the engine owns scheduling,
//! cancellation and terminal results.
use super::*;
use engine::jobs::{CancelToken, JobRunner, JobState};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;

/// One job file is bounded before any JSON allocation or project IO.
pub(crate) const MAX_JOB_FILE_BYTES: usize = 1 << 20;
/// Bounds the admitted output set and the retained project snapshots.
pub(crate) const MAX_BATCH_JOBS: usize = 1000;
/// Bounds aggregate distinct project bytes held as immutable snapshots.
pub(crate) const MAX_BATCH_PROJECT_BYTES: u64 = 64 << 20;
const MAX_JOB_PATH_BYTES: usize = control::MAX_PATH_BYTES;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobFile {
    version: u32,
    jobs: Vec<JobEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobEntry {
    project: String,
    output: String,
    #[serde(default)]
    aspect: AspectName,
    #[serde(default)]
    allow_degraded: bool,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AspectName {
    #[default]
    Project,
    #[serde(rename = "9:16")]
    Portrait,
    #[serde(rename = "16:9")]
    Landscape,
}
impl AspectName {
    fn cli(self) -> Option<Aspect> {
        match self {
            Self::Project => None,
            Self::Portrait => Some(Aspect::Portrait),
            Self::Landscape => Some(Aspect::Landscape),
        }
    }
}

struct PreparedJob {
    project: PathBuf,
    project_target: PathBuf,
    output: PathBuf,
    aspect: Option<Aspect>,
    allow_degraded: bool,
    overwrite: bool,
}

struct PreparedBatch {
    jobs: Vec<PreparedJob>,
    projects: BTreeMap<PathBuf, Arc<Project>>,
}

pub(crate) fn run(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut path = None;
    let mut allow_degraded = false;
    let mut allow_overwrite = false;
    for arg in args {
        match arg.to_str() {
            Some("--allow-degraded") => allow_degraded = true,
            Some("--allow-overwrite") => allow_overwrite = true,
            Some("--help" | "-h") => {
                return Ok(Output {
                    silent: false,
                    exit: 0,
                    value: json!({"usage": batch_usage_text()}),
                    human: batch_usage_text().to_owned(),
                });
            }
            Some(other) if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown batch flag: {other}")));
            }
            _ => {
                if path.is_some() {
                    return Err(CliError::Usage("batch takes one job-file path".into()));
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    let path = path.ok_or_else(batch_usage)?;
    let batch = load(&path, allow_degraded, allow_overwrite)?;
    execute(batch, engine::submit_export)
}

fn batch_usage_text() -> &'static str {
    "usage: vibecut-cli batch <job-file> [--allow-degraded] [--allow-overwrite]"
}
fn batch_usage() -> CliError {
    CliError::Usage(batch_usage_text().into())
}
fn invalid_job_file() -> CliError {
    CliError::Usage("invalid batch job file; see docs/product/cli.md#batch-job-files".into())
}

fn load(
    path: &Path,
    allow_degraded: bool,
    allow_overwrite: bool,
) -> Result<PreparedBatch, CliError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(CliError::Usage(
            "batch job file must be a regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_JOB_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JOB_FILE_BYTES {
        return Err(CliError::Engine(EditError::Limit));
    }
    let value = control::parse_frame(&bytes).map_err(|error| match error {
        EditError::Limit => CliError::Engine(EditError::Limit),
        _ => invalid_job_file(),
    })?;
    let file: JobFile = serde_json::from_value(value).map_err(|_| invalid_job_file())?;
    if file.version != 1 {
        return Err(CliError::Unsupported("unsupported batch job file version"));
    }
    if file.jobs.is_empty() {
        return Err(CliError::Usage(
            "batch job file must contain at least one job".into(),
        ));
    }
    if file.jobs.len() > MAX_BATCH_JOBS {
        return Err(CliError::Engine(EditError::Limit));
    }
    if file.jobs.iter().any(|job| job.overwrite) && !allow_overwrite {
        return Err(CliError::Rejected(
            "batch job requests overwrite without --allow-overwrite",
        ));
    }
    if file.jobs.iter().any(|job| job.allow_degraded) && !allow_degraded {
        return Err(CliError::Rejected(
            "batch job requests degraded output without --allow-degraded",
        ));
    }

    let base = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let base = fs::canonicalize(base)?;
    let job_file_target = destination_key(path)?;
    let mut jobs = Vec::new();
    jobs.try_reserve_exact(file.jobs.len())
        .map_err(|_| CliError::Engine(EditError::Limit))?;
    let mut destinations = BTreeSet::new();
    for entry in file.jobs {
        let project = resolve_path(&base, &entry.project, "project")?;
        let output = resolve_path(&base, &entry.output, "output")?;
        json_path(&project)?;
        json_path(&output)?;
        let parent = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !parent.is_dir() {
            return Err(CliError::Usage("batch output directory must exist".into()));
        }
        match fs::symlink_metadata(&output) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(CliError::Usage(
                    "batch output must be a regular file or absent".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        project::atomic::admit_destination(&output)?;
        if output.exists() && !entry.overwrite {
            return Err(CliError::Rejected(
                "batch output exists and the job does not allow overwrite",
            ));
        }
        let target = destination_key(&output)?;
        if !destinations.insert(target)
            || jobs
                .iter()
                .any(|job: &PreparedJob| same_file(&output, &job.output))
        {
            return Err(CliError::Usage("batch outputs must be distinct".into()));
        }
        let project_target = resolve_target(&project)?;
        jobs.push(PreparedJob {
            project,
            project_target,
            output,
            aspect: entry.aspect.cli(),
            allow_degraded: entry.allow_degraded,
            overwrite: entry.overwrite,
        });
    }

    let project_targets: BTreeSet<PathBuf> = jobs
        .iter()
        .map(|job| destination_key(&job.project))
        .collect::<Result<_, _>>()?;
    for job in &jobs {
        let target = destination_key(&job.output)?;
        if target == job_file_target || project_targets.contains(&target) {
            return Err(CliError::Usage(
                "batch output aliases a project or the job file".into(),
            ));
        }
    }

    let mut projects = BTreeMap::new();
    let mut project_bytes = 0u64;
    for job in &jobs {
        if projects.contains_key(&job.project_target) {
            continue;
        }
        project_bytes = add_project_bytes(project_bytes, fs::metadata(&job.project)?.len())?;
        let loaded = project::load(&job.project)?;
        projects.insert(job.project_target.clone(), Arc::new(loaded.project));
    }
    let mut sources = BTreeSet::new();
    let mut protected = vec![path.to_path_buf()];
    protected.extend(jobs.iter().map(|job| job.project.clone()));
    for job in &jobs {
        if let Ok(metadata) = project::metadata_path(&job.project) {
            if let Ok(target) = destination_key(&metadata) {
                sources.insert(target);
            }
            sources.insert(metadata.clone());
            protected.push(metadata);
        }
    }
    for project in projects.values() {
        for asset in &project.data().assets {
            if let Ok(target) = destination_key(asset.path()) {
                sources.insert(target);
            }
            sources.insert(asset.path().to_path_buf());
            protected.push(asset.path().to_path_buf());
        }
    }
    for job in &jobs {
        let target = destination_key(&job.output)?;
        if sources.contains(&target)
            || sources.contains(&job.output)
            || protected.iter().any(|path| same_file(&job.output, path))
        {
            return Err(CliError::Usage(
                "batch output aliases a project or source".into(),
            ));
        }
    }
    Ok(PreparedBatch { jobs, projects })
}

fn add_project_bytes(total: u64, size: u64) -> Result<u64, CliError> {
    total
        .checked_add(size)
        .filter(|total| *total <= MAX_BATCH_PROJECT_BYTES)
        .ok_or(CliError::Engine(EditError::Limit))
}

fn same_file(left: &Path, right: &Path) -> bool {
    same_file::is_same_file(left, right).unwrap_or(false)
}

// Canonicalization cannot fold a basename that does not exist yet. Refuse
// case-only Windows/macOS collisions conservatively, including on case-sensitive
// directories; this key is only for protection, never for snapshot selection.
fn destination_key(path: &Path) -> Result<PathBuf, CliError> {
    let target = resolve_target(path)?;
    #[cfg(any(windows, target_os = "macos"))]
    let target = PathBuf::from(json_path(&target)?.to_uppercase());
    // Default macOS volumes also equate composed/decomposed spellings. This is
    // only a conservative protection key; actual native paths stay unchanged.
    #[cfg(target_os = "macos")]
    let target = PathBuf::from(String::from(NfcText::from(json_path(&target)?)));
    Ok(target)
}

fn resolve_path(base: &Path, raw: &str, what: &str) -> Result<PathBuf, CliError> {
    if raw.is_empty() {
        return Err(CliError::Usage(format!(
            "batch {what} path must not be empty"
        )));
    }
    if raw.len() > MAX_JOB_PATH_BYTES {
        return Err(CliError::Engine(EditError::Limit));
    }
    if raw.contains('\0') {
        return Err(CliError::Usage(format!(
            "batch {what} path must not contain NUL"
        )));
    }
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    if path.file_name().is_none() {
        return Err(CliError::Usage(format!(
            "batch {what} path must name a file"
        )));
    }
    Ok(path)
}

fn execute<F>(batch: PreparedBatch, mut submit: F) -> Result<Output, CliError>
where
    F: FnMut(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, CancelToken), EditError>,
{
    let budget = ByteBudget::new(CLI_MEMORY_LIMIT)?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(20),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?;
    let total = batch.jobs.len();
    let mut values = Vec::new();
    values
        .try_reserve_exact(total)
        .map_err(|_| CliError::Engine(EditError::Limit))?;
    let mut human = Vec::new();
    let mut exit = 0;
    for (index, job) in batch.jobs.iter().enumerate() {
        if exit != 0 {
            values.push(job_value(job, index, "skipped", None)?);
            human.push(format!("job {}/{}: skipped", index + 1, total));
            continue;
        }
        let project = batch
            .projects
            .get(&job.project_target)
            .cloned()
            .ok_or(CliError::Rejected("batch project disappeared"))?;
        let outcome = run_job(&mut runner, job, project, index, total, &mut submit);
        match outcome {
            Ok(()) => {
                values.push(job_value(job, index, "completed", None)?);
                human.push(format!(
                    "job {}/{}: exported {}",
                    index + 1,
                    total,
                    job.output.display()
                ));
            }
            Err(error) => {
                exit = error.exit_code();
                values.push(job_value(job, index, "failed", Some(&error))?);
                human.push(format!("job {}/{}: failed: {error}", index + 1, total));
            }
        }
    }
    runner.shutdown();
    let status = if exit == 0 { "completed" } else { "failed" };
    Ok(Output {
        value: json!({"status": status, "jobs": values}),
        human: human.join("\n"),
        silent: false,
        exit,
    })
}

fn job_value(
    job: &PreparedJob,
    index: usize,
    status: &'static str,
    error: Option<&CliError>,
) -> Result<Value, CliError> {
    let mut value = json!({
        "index": index,
        "project": json_path(&job.project)?,
        "output": json_path(&job.output)?,
        "status": status,
    });
    if let Some(error) = error
        && let Value::Object(fields) = &mut value
    {
        let _ = fields.insert("error".into(), error.value());
    }
    Ok(value)
}

fn run_job<F>(
    runner: &mut JobRunner,
    job: &PreparedJob,
    project: Arc<Project>,
    index: usize,
    total: usize,
    submit: &mut F,
) -> Result<(), CliError>
where
    F: FnMut(
        &mut JobRunner,
        CacheKey,
        Arc<Project>,
        TimeRange,
        render::ExportSettings,
        PathBuf,
    ) -> Result<(JobId, CancelToken), EditError>,
{
    let duration = project.duration();
    if duration <= Tick::ZERO {
        return Err(CliError::Rejected("project timeline is empty"));
    }
    let range = TimeRange::new(Tick::ZERO, duration)?;
    let settings = export_settings(&project, job.aspect, job.allow_degraded, job.overwrite);
    let key = export_job_key(&project, range, &settings)?;
    let (id, _cancel) = submit(runner, key, project, range, settings, job.output.clone())?;
    wait_for_job(runner, id, index, total)
}

fn wait_for_job(
    runner: &mut JobRunner,
    id: JobId,
    index: usize,
    total: usize,
) -> Result<(), CliError> {
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
                    "job {}/{}: progress {} {}%",
                    index + 1,
                    total,
                    stage_name(event.stage),
                    percent
                );
            }
        }
        let snapshot = runner
            .snapshot(id)
            .ok_or(CliError::Rejected("export job disappeared"))?;
        match snapshot.state {
            JobState::Completed => {
                let _ = runner.take_output(id);
                return Ok(());
            }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use project::{Item, ItemId, ItemParams, OverlayParams, Project, Track, TrackId, TrackKind};

    fn overlay_project() -> Project {
        let mut data = Project::empty(ProjectId(0), "batch".into())
            .unwrap()
            .into_data();
        data.canvas.width = 64;
        data.canvas.height = 64;
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(1),
            name: "Overlay".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![Item {
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: ItemId(0),
                source_asset: None,
                source_range: TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Overlay(OverlayParams {
                    color: project::Color([30, 160, 90, 255]),
                }),
            }],
        });
        Project::new(data).unwrap()
    }

    #[test]
    fn project_snapshot_bytes_admit_the_limit_and_refuse_one_more() {
        assert_eq!(
            add_project_bytes(MAX_BATCH_PROJECT_BYTES - 1, 1).unwrap(),
            MAX_BATCH_PROJECT_BYTES
        );
        assert_eq!(
            add_project_bytes(0, MAX_BATCH_PROJECT_BYTES).unwrap(),
            MAX_BATCH_PROJECT_BYTES
        );
        assert_eq!(
            add_project_bytes(MAX_BATCH_PROJECT_BYTES, 1)
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn cancellation_leaves_no_partial_output() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().join("project.vibecut");
        let output_path = dir.path().join("output.mp4");
        let job = PreparedJob {
            project: project_path.clone(),
            project_target: project_path.clone(),
            output: output_path.clone(),
            aspect: None,
            allow_degraded: false,
            overwrite: false,
        };
        let mut projects = BTreeMap::new();
        projects.insert(project_path, Arc::new(overlay_project()));
        let batch = PreparedBatch {
            jobs: vec![job],
            projects,
        };
        let result = execute(batch, |runner, key, project, range, settings, path| {
            let (id, token) = engine::submit_export(runner, key, project, range, settings, path)?;
            token.cancel();
            Ok((id, token))
        })
        .unwrap();
        assert_eq!(result.exit_code(), 7);
        assert_eq!(result.value["status"], "failed");
        assert_eq!(result.value["jobs"][0]["status"], "failed");
        assert!(!output_path.exists());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().starts_with(".vibecut-export-"))
            .collect();
        assert!(leftovers.is_empty(), "staging orphan: {leftovers:?}");
    }
}

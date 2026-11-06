//! Admits a bounded batch file before export work.
use super::{
    JobFile, MAX_BATCH_JOBS, MAX_BATCH_PROJECT_BYTES, MAX_JOB_FILE_BYTES, MAX_JOB_PATH_BYTES,
    PreparedBatch, PreparedJob,
};
use crate::CliError;
use crate::output::json_path;
use crate::session::resolve_target;
use engine::control;
use engine::{EditError, NfcText};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn invalid_job_file() -> CliError {
    CliError::Usage("invalid batch job file; see docs/product/cli.md#batch-job-files".into())
}

pub(super) fn load(
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

pub(super) fn add_project_bytes(total: u64, size: u64) -> Result<u64, CliError> {
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

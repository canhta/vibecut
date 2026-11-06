//! Executes a bounded file of export requests.
//!
//! A job file names projects and destinations only. The batch admits the request, loads each
//! project once as an immutable snapshot, and submits the existing `engine::submit_export` jobs
//! in file order. Render owns pixels, staging and the publication fence; the engine owns
//! scheduling, cancellation and terminal results.
mod admission;
mod execution;
mod request;
use crate::session::Aspect;
use admission::load;
use engine::control;
use execution::execute;
use project::Project;
pub(crate) use request::run;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// One job file is bounded before any JSON allocation or project IO.
const MAX_JOB_FILE_BYTES: usize = 1 << 20;
/// Bounds the admitted output set and the retained project snapshots.
const MAX_BATCH_JOBS: usize = 1000;
/// Bounds aggregate distinct project bytes held as immutable snapshots.
const MAX_BATCH_PROJECT_BYTES: u64 = 64 << 20;
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

#[cfg(test)]
mod limits {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::{MAX_BATCH_PROJECT_BYTES, admission::add_project_bytes};
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
}

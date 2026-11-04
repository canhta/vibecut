use crate::cache::CacheKey;
use crate::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use crate::{ByteLease, ByteOwner, EditError};
use std::{path::PathBuf, sync::Arc};
impl render::Reservation for ByteLease {}
impl render::ExecutionHooks for JobContext {
    fn cancelled(&self) -> bool {
        self.cancelled()
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        let owner = match kind {
            render::MemoryKind::Decoded => ByteOwner::DecodedFrames,
            render::MemoryKind::Compositor => ByteOwner::Compositor,
            render::MemoryKind::Export => ByteOwner::Export,
        };
        self.memory_budget()
            .reserve(owner, bytes)
            .map(|lease| Box::new(lease) as Box<dyn render::Reservation>)
            .map_err(|error| match error {
                EditError::Io(source) => render::Error::Io(source),
                _ => render::Error::Budget {
                    requested: bytes,
                    limit: self
                        .memory_budget()
                        .snapshot()
                        .map_or(0, |s| s.limit.saturating_sub(s.used)),
                },
            })
    }
    fn begin_publish(&self) -> render::Result<()> {
        self.begin_publication()
            .map_err(|_| render::Error::Cancelled)
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        self.progress(
            match stage {
                render::Stage::Admit => JobStage::Admit,
                render::Stage::Render => JobStage::Render,
                render::Stage::Mix => JobStage::Mix,
                render::Stage::Encode => JobStage::Encode,
                render::Stage::Finalize | render::Stage::Finalized => JobStage::Finalize,
                render::Stage::Publish => JobStage::Publish,
            },
            fraction,
        )
        .map_err(|e| {
            if matches!(e, EditError::Cancelled) {
                render::Error::Cancelled
            } else {
                render::Error::Invalid("job progress")
            }
        })
    }
}
/// Export the captured immutable project through the production render path.
/// Render reserves its actual buffer lifetimes through this JobContext; the task
/// declares zero additional working bytes to avoid charging each buffer twice.
/// The output is a completed file, not an edit applied to the current Session.
pub fn submit_export(
    runner: &mut JobRunner,
    input: CacheKey,
    project: Arc<project::Project>,
    range: time::TimeRange,
    settings: render::ExportSettings,
    path: PathBuf,
) -> Result<(JobId, CancelToken), EditError> {
    runner.submit(
        JobSpec {
            kind: JobKind::Export,
            class: ResourceClass::GpuEncoder,
            input,
            bytes: JobBytes::default(),
        },
        move |context| {
            render::export(&project, range, &settings, &path, context)?;
            Ok(Vec::new())
        },
    )
}

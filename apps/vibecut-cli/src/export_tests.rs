#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use engine::jobs::{JobBytes, JobContext, JobKind, JobSpec, ResourceClass};
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "../tests/it/common/mod.rs"]
mod common;

struct SourceLoss<'a> {
    context: &'a JobContext,
    path: &'a Path,
    removed: &'a AtomicBool,
}
impl render::ExecutionHooks for SourceLoss<'_> {
    fn cancelled(&self) -> bool {
        self.context.cancelled()
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        render::ExecutionHooks::reserve(self.context, kind, bytes)
    }
    fn progress(&self, stage: render::Stage, fraction: f32) -> render::Result<()> {
        // Gate execution after admission and the first decoded/rendered frame.
        // Latest-only, throttled JobRunner progress cannot synchronize a test.
        if stage == render::Stage::Render && !self.removed.swap(true, Ordering::SeqCst) {
            std::fs::remove_file(self.path)?;
        }
        render::ExecutionHooks::progress(self.context, stage, fraction)
    }
    fn begin_publish(&self) -> render::Result<()> {
        render::ExecutionHooks::begin_publish(self.context)
    }
}

#[test]
fn export_refuses_when_a_source_disappears_mid_job() {
    if !common::tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("midjob.mp4");
    let project = dir.path().join("midjob.vibecut");
    let output = dir.path().join("midjob-out.mp4");
    common::generate_av(&input);
    common::media_project(&project, &input, None);
    std::fs::write(&output, b"previous").unwrap();
    let removed = Arc::new(AtomicBool::new(false));
    let signal = removed.clone();
    let result = export_project_with(
        project,
        output.clone(),
        None,
        false,
        true,
        move |runner, input_key, project, range, mut settings, path| {
            settings.mode = render::ExportMode::Encode;
            runner.submit(
                JobSpec {
                    kind: JobKind::Export,
                    class: ResourceClass::GpuEncoder,
                    input: input_key,
                    bytes: JobBytes::default(),
                },
                move |context| {
                    render::export(
                        &project,
                        range,
                        &settings,
                        &path,
                        &SourceLoss {
                            context,
                            path: &input,
                            removed: &signal,
                        },
                    )?;
                    Ok(Vec::new())
                },
            )
        },
    );
    assert!(
        removed.load(Ordering::SeqCst),
        "render gate was not reached"
    );
    let error = result.err().expect("source loss must refuse publication");
    assert_eq!(error.exit_code(), 4);
    assert!(matches!(
        error,
        CliError::Job(error) if matches!(error.as_ref(),
            EditError::Render(render::Error::Unavailable(entries))
            if entries.iter().any(|entry| entry.asset == media::AssetId(1)
                && entry.item == project::ItemId(0)
                && entry.reason == render::UnavailableReason::Missing))
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

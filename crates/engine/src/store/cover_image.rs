//! Captures a managed project cover image.
use project::{Project, Thumbnail};
use std::sync::Arc;
use time::Tick;

/// Reads a project's stored footage corrections from the process media cache. A store that
/// renders cover images holds one, lent by the composition root, so covers draw the same
/// correction the preview and export draw.
pub type Lookup = Arc<dyn Fn(&Project) -> render::Corrections + Send + Sync>;

/// The lookup of a process with no media cache: every cover renders uncorrected.
pub fn uncorrected() -> Lookup {
    Arc::new(|_: &Project| render::Corrections::default())
}

/// Sample the active scene at the cover tick; preview/playhead state is irrelevant. A tick at or
/// past the project's end samples output zero. The stored footage corrections come from
/// `corrections`, as the preview and export draw them.
pub(crate) fn capture(
    project: &Project,
    tick: Tick,
    hooks: &dyn render::ExecutionHooks,
    corrections: &Lookup,
) -> Thumbnail {
    fn render(
        project: &Project,
        tick: Tick,
        hooks: &dyn render::ExecutionHooks,
        corrections: &Lookup,
    ) -> render::Result<Thumbnail> {
        let canvas = &project.data().canvas;
        let edge = u64::from(canvas.width.max(canvas.height));
        let mut settings = render::RenderSettings::from_project(project);
        settings.width = (u64::from(canvas.width) * 128 / edge).max(1) as u32;
        settings.height = (u64::from(canvas.height) * 128 / edge).max(1) as u32;
        corrections(project).attach(&mut settings);
        let (width, height) = (settings.width, settings.height);
        let mut renderer = render::Renderer::new(project, settings)?;
        let image =
            renderer.render_frame(tick, &mut render::FileSources::default(), hooks)?;
        let rgba = image.video_rgba8(13, hooks)?.bytes;
        Ok(Thumbnail::Ready {
            width,
            height,
            rgba,
        })
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let tick = if tick < project.duration() {
            tick
        } else {
            Tick::ZERO
        };
        render(project, tick, hooks, corrections)
    }))
    .ok()
    .and_then(Result::ok)
    .unwrap_or(Thumbnail::Unavailable)
}

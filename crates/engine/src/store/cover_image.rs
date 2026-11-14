//! Captures a managed project cover image.
use crate::footage::Lookup;
use project::{Project, Thumbnail};
use time::Tick;

/// Always sample active-scene output zero; preview/playhead state is irrelevant. The stored
/// footage corrections come from `corrections`, as the preview and export draw them.
pub(crate) fn capture(
    project: &Project,
    hooks: &dyn render::ExecutionHooks,
    corrections: &Lookup,
) -> Thumbnail {
    fn render(
        project: &Project,
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
            renderer.render_frame(Tick::ZERO, &mut render::FileSources::default(), hooks)?;
        let rgba = image.video_rgba8(13, hooks)?.bytes;
        Ok(Thumbnail::Ready {
            width,
            height,
            rgba,
        })
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render(project, hooks, corrections)
    }))
    .ok()
    .and_then(Result::ok)
    .unwrap_or(Thumbnail::Unavailable)
}

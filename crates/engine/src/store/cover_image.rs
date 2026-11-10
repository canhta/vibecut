//! Captures a managed project cover image.
use project::{Project, Thumbnail};
use time::Tick;

/// Always sample active-scene output zero; preview/playhead state is irrelevant.
pub(crate) fn capture(project: &Project, hooks: &dyn render::ExecutionHooks) -> Thumbnail {
    fn render(project: &Project, hooks: &dyn render::ExecutionHooks) -> render::Result<Thumbnail> {
        let canvas = &project.data().canvas;
        let edge = u64::from(canvas.width.max(canvas.height));
        let mut settings = render::RenderSettings::from_project(project);
        settings.width = (u64::from(canvas.width) * 128 / edge).max(1) as u32;
        settings.height = (u64::from(canvas.height) * 128 / edge).max(1) as u32;
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
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(project, hooks)))
        .ok()
        .and_then(Result::ok)
        .unwrap_or(Thumbnail::Unavailable)
}

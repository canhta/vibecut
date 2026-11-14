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

/// A cover frame as straight sRGB RGBA8 at the size it was asked for.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Render the active scene at the cover tick at `width` x `height`; preview/playhead state is
/// irrelevant. A tick at or past the project's end renders output zero. `corrections` are the
/// stored footage corrections, as the preview and export draw them. Adapters that show the cover
/// larger than the 128 px stored thumbnail render it here at their own size.
pub fn render(
    project: &Project,
    tick: Tick,
    [width, height]: [u32; 2],
    hooks: &dyn render::ExecutionHooks,
    corrections: &render::Corrections,
) -> render::Result<Image> {
    let tick = if tick < project.duration() {
        tick
    } else {
        Tick::ZERO
    };
    let mut settings = render::RenderSettings::from_project(project);
    settings.width = width.max(1);
    settings.height = height.max(1);
    corrections.clone().attach(&mut settings);
    let (width, height) = (settings.width, settings.height);
    let mut renderer = render::Renderer::new(project, settings)?;
    let image = renderer.render_frame(tick, &mut render::FileSources::default(), hooks)?;
    let rgba = image.video_rgba8(13, hooks)?.bytes;
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// Capture the stored cover thumbnail: the cover frame at most 128 pixels per edge. The stored
/// footage corrections come from `corrections`.
pub(crate) fn capture(
    project: &Project,
    tick: Tick,
    hooks: &dyn render::ExecutionHooks,
    corrections: &Lookup,
) -> Thumbnail {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let canvas = &project.data().canvas;
        let edge = u64::from(canvas.width.max(canvas.height));
        let size = [
            (u64::from(canvas.width) * 128 / edge).max(1) as u32,
            (u64::from(canvas.height) * 128 / edge).max(1) as u32,
        ];
        render(project, tick, size, hooks, &corrections(project)).map(|image| Thumbnail::Ready {
            width: image.width,
            height: image.height,
            rgba: image.rgba,
        })
    }))
    .ok()
    .and_then(Result::ok)
    .unwrap_or(Thumbnail::Unavailable)
}

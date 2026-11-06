//! Captures a managed project cover image.
use crate::{ByteBudget, ByteOwner};
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

pub(crate) struct Hooks<'a>(pub &'a ByteBudget);
impl render::ExecutionHooks for Hooks<'_> {
    fn cancelled(&self) -> bool {
        false
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
        self.0
            .reserve(owner, bytes)
            .map(|lease| Box::new(lease) as Box<dyn render::Reservation>)
            .map_err(|_| render::Error::Budget {
                requested: bytes,
                limit: 0,
            })
    }
}

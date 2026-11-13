//! Measures the person matte of every frame of one source with the lent segmenter.
use super::{CorrectionRequest, MAX_PIXELS, Segmenter};
use crate::analysis::{Error as AnalysisError, Result, VisionHooks};
use crate::jobs::{JobContext, JobStage};
use media::VisionPrepLimits;
use render::matte::{MatteBuilder, MatteTrack};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

impl CorrectionRequest {
    /// Decode the whole video once, in slices of at most 300 s, and store the segmenter's plane of
    /// every frame. Memory is one decoded frame, one 192x192 input and the compressed planes so
    /// far (at most `render::matte::MAX_BYTES`); cancellation is checked per frame and inside the
    /// segmenter.
    pub(super) fn analyze_matte(
        &self,
        ctx: &JobContext,
        segmenter: &mut Segmenter,
    ) -> Result<MatteTrack> {
        let hooks = VisionHooks(ctx);
        let mut builder = MatteBuilder::default();
        let mut last: Option<Tick> = None;
        let total = self.range.duration().0.max(1) as f64;
        let mut from = self.range.start();
        while from < self.range.end() {
            ctx.check_cancelled()?;
            let end = from
                .saturating_add(Tick(300 * TICKS_PER_SECOND))
                .min(self.range.end());
            let mut failure: Option<AnalysisError> = None;
            let decoded = media::decode_shot_frames(
                &self.asset,
                SourceRange::new(from, end)?,
                &VisionPrepLimits {
                    max_frame_pixels: MAX_PIXELS,
                    ..Default::default()
                },
                ctx.media_cancel_flag(),
                |frame| {
                    if ctx.cancelled() {
                        return Err(media::MediaError::Cancelled);
                    }
                    let pts = frame.source_pts();
                    // The frame covering a slice start may already have ended the last slice.
                    if last.is_some_and(|last| last >= pts) {
                        return Ok(());
                    }
                    let tensor = frame.person_tensor()?;
                    let plane = ai::vision::PersonFrame::new(&tensor)
                        .and_then(|input| segmenter(input, &hooks));
                    let plane = match plane {
                        Ok(plane) => plane,
                        Err(error) => {
                            failure = Some(AnalysisError::Ai(error));
                            return Err(media::MediaError::Invalid("person matte frame"));
                        }
                    };
                    if let Err(error) = builder.push(pts, &plane.quantized()) {
                        failure = Some(AnalysisError::Edit(
                            crate::session::EditError::Unsupported(error.0),
                        ));
                        return Err(media::MediaError::TooLarge("person matte"));
                    }
                    last = Some(pts);
                    ctx.progress(
                        JobStage::Analyzing,
                        ((pts.saturating_sub(self.range.start()).0.max(0) as f64 / total) as f32)
                            .clamp(0.0, 1.0),
                    )
                    .map_err(|_| media::MediaError::Cancelled)
                },
            );
            if let Some(error) = failure {
                return Err(error);
            }
            decoded?;
            from = end;
        }
        if builder.is_empty() {
            return Err(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "footage correction has no frames",
            )));
        }
        builder.finish().map_err(|_| {
            AnalysisError::Edit(crate::session::EditError::Invalid("person matte track"))
        })
    }
}

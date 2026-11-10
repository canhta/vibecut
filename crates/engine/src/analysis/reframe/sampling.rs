//! Samples a shot at 8 Hz and follows face boxes across the samples.
use super::STEP;
use crate::{
    EditError,
    analysis::{Error as AnalysisError, Result, VisionHooks, iou},
    jobs::{JobContext, JobStage},
    session::wire::ControlError,
};
use ai::vision::{ActiveSpeaker, FaceBox, FaceFrame, SceneResult, ShotResult, VisionDetector};
use media::{SourceAsset, VisionFrame, VisionPrepLimits};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// A lost face is held, and its track kept, for 0.5 s of samples.
pub(super) const HOLD: i64 = 4;
const MAX_TRACKS: usize = 64;
const MAX_PIXELS: u64 = 1 << 24;
const SLICE: i64 = 300 * TICKS_PER_SECOND;
/// Association threshold of the measured R1 harness: a face moves at most about
/// half its width between two samples.
const IOU: f32 = 0.2;

/// One detection of a track, in source pixels.
pub(super) struct Hit {
    pub sample: usize,
    pub cx: f32,
    pub cy: f32,
    pub area: f32,
}
pub(super) struct Track {
    pub hits: Vec<Hit>,
    last: FaceBox,
    seen: i64,
}
pub(super) struct Observed {
    pub samples: usize,
    pub tracks: Vec<Track>,
}
struct Sampler<'a> {
    shot: SourceRange,
    whole: SourceRange,
    samples: usize,
    next: usize,
    tracks: Vec<Track>,
    detector: &'a mut dyn VisionDetector,
    ctx: &'a JobContext,
}
/// Detects faces at every 1/8 s of the shot. Each sample uses the decoded frame
/// whose presentation interval contains it; one retained frame and one tensor
/// are resident at a time. Samples with no covering frame see no face.
pub(super) fn observe(
    asset: &SourceAsset,
    shot: SourceRange,
    whole: SourceRange,
    detector: &mut dyn VisionDetector,
    ctx: &JobContext,
) -> Result<Observed> {
    let samples = ((shot.duration().0 - 1) / STEP + 1) as usize;
    let mut sampler = Sampler {
        shot,
        whole,
        samples,
        next: 0,
        tracks: Vec::new(),
        detector,
        ctx,
    };
    let mut start = shot.start();
    let mut decoded = 0usize;
    while start < shot.end() {
        let end = Tick(start.0.saturating_add(SLICE).min(shot.end().0));
        let mut previous: Option<VisionFrame> = None;
        let mut failure = None;
        let result = media::decode_shot_frames(
            asset,
            SourceRange::new(start, end)?,
            &VisionPrepLimits {
                max_frame_pixels: MAX_PIXELS,
                ..Default::default()
            },
            ctx.media_cancel_flag(),
            |frame| {
                if ctx.cancelled() {
                    return Err(media::MediaError::Cancelled);
                }
                decoded += 1;
                if let Some(held) = previous.take()
                    && let Err(e) = sampler.flush(&held, frame.source_pts())
                {
                    failure = Some(e);
                    return Err(media::MediaError::Invalid("reframe frame"));
                }
                previous = Some(frame);
                Ok(())
            },
        );
        if let Some(e) = failure {
            return Err(e);
        }
        result?;
        if let Some(held) = previous {
            sampler.flush(&held, end)?;
        }
        start = end;
    }
    if decoded == 0 {
        return Err(AnalysisError::from(EditError::Unsupported(
            "reframe shot has no frames",
        )));
    }
    Ok(Observed {
        samples,
        tracks: sampler.tracks,
    })
}
impl Sampler<'_> {
    fn time(&self, sample: usize) -> Tick {
        Tick(self.shot.start().0 + sample as i64 * STEP)
    }
    /// Takes every pending sample before `until` from `frame`.
    fn flush(&mut self, frame: &VisionFrame, until: Tick) -> Result<()> {
        while self.next < self.samples && self.time(self.next) < until {
            self.ctx.check_cancelled()?;
            if self.time(self.next) >= frame.source_pts() {
                self.detect(frame)?;
            }
            self.next += 1;
            let done = self.time(self.next).0 - self.whole.start().0;
            self.ctx.progress(
                JobStage::Analyzing,
                (done as f64 / self.whole.duration().0 as f64).clamp(0., 1.) as f32,
            )?;
        }
        Ok(())
    }
    fn detect(&mut self, frame: &VisionFrame) -> Result<()> {
        let tensor = frame.face_tensor()?;
        let faces = self
            .detector
            .faces(FaceFrame::new(tensor.values())?, &VisionHooks(self.ctx))?;
        // Validate injected adapters as strictly as the production adapter.
        ShotResult::new(
            faces.clone(),
            ActiveSpeaker::NoVisibleSpeaker,
            SceneResult::unknown(),
        )?;
        let mut faces: Vec<_> = faces
            .into_iter()
            .filter(|f| {
                f.x < tensor.resized_width() as f32
                    && f.y < tensor.resized_height() as f32
                    && f.x + f.width > 0.
                    && f.y + f.height > 0.
            })
            .collect();
        faces.sort_by(|a, b| {
            a.x.total_cmp(&b.x)
                .then(a.y.total_cmp(&b.y))
                .then(b.confidence.total_cmp(&a.confidence))
        });
        let sample = self.next as i64;
        let sx = frame.width() as f32 / tensor.resized_width() as f32;
        let sy = frame.height() as f32 / tensor.resized_height() as f32;
        for face in faces {
            let bounds = FaceBox {
                landmarks: None,
                x: face.x * sx,
                y: face.y * sy,
                width: face.width * sx,
                height: face.height * sy,
                confidence: face.confidence,
            };
            let best = self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.seen != sample && sample - t.seen <= HOLD)
                .map(|(i, t)| (i, iou(&t.last, &bounds)))
                .filter(|(_, score)| *score >= IOU)
                .max_by(|(a, x), (b, y)| x.total_cmp(y).then(b.cmp(a)))
                .map(|(i, _)| i);
            let hit = Hit {
                sample: self.next,
                cx: bounds.x + bounds.width / 2.,
                cy: bounds.y + bounds.height / 2.,
                area: bounds.width * bounds.height,
            };
            if let Some(i) = best {
                let track = self.tracks.get_mut(i).ok_or(ControlError::Limit)?;
                track.last = bounds;
                track.seen = sample;
                track.hits.push(hit);
            } else {
                if self.tracks.len() == MAX_TRACKS {
                    return Err(ControlError::Limit.into());
                }
                self.tracks.push(Track {
                    hits: vec![hit],
                    last: bounds,
                    seen: sample,
                });
            }
        }
        Ok(())
    }
}

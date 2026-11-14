//! Measures the brightness of every frame of one source and where its shots change.
use super::{CorrectionRequest, MAX_PIXELS};
use crate::analysis::{Error as AnalysisError, Result};
use crate::jobs::{JobContext, JobStage};
use media::VisionPrepLimits;
use render::flicker::{CUT_THRESHOLD, DARK, FlickerTrack, GRID, MAX_FRAMES};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// One unit of the integer linear luma: 256 weight units of a 16-bit linear sample.
const UNIT: f64 = 256.0 * 65535.0;

/// A frame stream measured for its mean linear luma and, against its predecessor, for a shot
/// change. Sums are integers over a table of 16-bit linear samples, so a frame always gives the
/// same mean; the cut score is `f64` in a fixed order.
pub(super) struct Luma {
    source: (usize, usize),
    grid: (usize, usize),
    /// Linear light of each 8-bit encoded value, 0..=65535.
    linear: [u16; 256],
    /// The previous frame's grid cells and their mean, `None` before the first frame.
    previous: Option<(Vec<f64>, f64)>,
}
impl Luma {
    /// A stream of `width` x `height` frames, or `None` for an empty frame.
    pub(super) fn new(width: u32, height: u32) -> Option<Self> {
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let mut linear = [0u16; 256];
        for (value, slot) in linear.iter_mut().enumerate() {
            let encoded = value as f64 / 255.0;
            let light = if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            };
            *slot = (light * 65535.0).round() as u16;
        }
        Some(Self {
            source: (width, height),
            grid: (GRID[0].min(width), GRID[1].min(height)),
            linear,
            previous: None,
        })
    }
    /// Feed the next frame, packed RGBA8, and get its mean linear luma and whether it starts a
    /// new shot. `None` when the frame is not the size this stream started with.
    pub(super) fn push(&mut self, rgba: &[u8], width: u32, height: u32) -> Option<(f32, bool)> {
        let (source_width, source_height) = self.source;
        if (width as usize, height as usize) != self.source
            || rgba.len() != source_width * source_height * 4
        {
            return None;
        }
        let (grid_width, grid_height) = self.grid;
        let mut sums = vec![0u64; grid_width * grid_height];
        let mut counts = vec![0u64; grid_width * grid_height];
        let mut total = 0u64;
        for (y, row) in rgba.chunks_exact(source_width * 4).enumerate() {
            let line = y * grid_height / source_height * grid_width;
            for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
                let [r, g, b, _] = *pixel;
                let luma = 54 * u64::from(*self.linear.get(usize::from(r))?)
                    + 183 * u64::from(*self.linear.get(usize::from(g))?)
                    + 19 * u64::from(*self.linear.get(usize::from(b))?);
                total += luma;
                let cell = line + x * grid_width / source_width;
                *sums.get_mut(cell)? += luma;
                *counts.get_mut(cell)? += 1;
            }
        }
        let mean = total as f64 / UNIT / (source_width * source_height) as f64;
        let cells: Vec<f64> = sums
            .iter()
            .zip(&counts)
            .map(|(sum, count)| *sum as f64 / UNIT / (*count).max(1) as f64)
            .collect();
        let cells_mean = cells.iter().sum::<f64>() / cells.len().max(1) as f64;
        let starts_shot = match &self.previous {
            None => false,
            Some((before, before_mean)) => shot_changed(before, *before_mean, &cells, cells_mean),
        };
        self.previous = Some((cells, cells_mean));
        Some((mean as f32, starts_shot))
    }
}

/// Whether two consecutive frames are different shots: the mean absolute difference of their
/// mean-normalised grids is above the threshold, so a pure gain change never counts. A dark
/// frame has no normalisation: black next to a lit frame is a cut, two dark frames are not.
fn shot_changed(before: &[f64], before_mean: f64, after: &[f64], after_mean: f64) -> bool {
    let dark = f64::from(DARK);
    match (before_mean < dark, after_mean < dark) {
        (true, true) => false,
        (true, false) | (false, true) => true,
        (false, false) => {
            let sum: f64 = before
                .iter()
                .zip(after)
                .map(|(a, b)| (a / before_mean - b / after_mean).abs())
                .sum();
            sum / before.len().max(1) as f64 > CUT_THRESHOLD
        }
    }
}

impl CorrectionRequest {
    /// Decode the whole video once, in slices of at most 300 s, and measure every frame. Memory
    /// is one decoded frame, two grids and 13 bytes per frame; cancellation is checked per frame.
    pub(super) fn analyze_flicker(&self, ctx: &JobContext) -> Result<FlickerTrack> {
        let video = self
            .asset
            .probe()
            .streams
            .iter()
            .find_map(|s| s.video.as_ref())
            .ok_or(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "footage correction needs video",
            )))?;
        let mut luma = Luma::new(video.width, video.height).ok_or(AnalysisError::Edit(
            crate::session::EditError::Unsupported("footage correction frame size"),
        ))?;
        let mut times: Vec<Tick> = Vec::new();
        let mut means = Vec::new();
        let mut cuts = Vec::new();
        let total = self.range.duration().0.max(1) as f64;
        let mut from = self.range.start();
        while from < self.range.end() {
            ctx.check_cancelled()?;
            let end = from
                .saturating_add(Tick(300 * TICKS_PER_SECOND))
                .min(self.range.end());
            media::decode_shot_frames(
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
                    if times.last().is_some_and(|last| *last >= pts) {
                        return Ok(());
                    }
                    if times.len() >= MAX_FRAMES {
                        return Err(media::MediaError::TooLarge("footage correction frames"));
                    }
                    let (mean, starts_shot) = luma
                        .push(frame.rgba(), frame.width(), frame.height())
                        .ok_or(media::MediaError::Invalid("footage correction frame size"))?;
                    times.push(pts);
                    means.push(mean);
                    cuts.push(starts_shot);
                    ctx.progress(
                        JobStage::Analyzing,
                        ((pts.saturating_sub(self.range.start()).0.max(0) as f64 / total) as f32)
                            .clamp(0.0, 1.0),
                    )
                    .map_err(|_| media::MediaError::Cancelled)
                },
            )?;
            from = end;
        }
        if times.is_empty() {
            return Err(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "footage correction has no frames",
            )));
        }
        FlickerTrack::new(times, means, cuts).map_err(|_| {
            AnalysisError::Edit(crate::session::EditError::Invalid(
                "footage correction track",
            ))
        })
    }
}

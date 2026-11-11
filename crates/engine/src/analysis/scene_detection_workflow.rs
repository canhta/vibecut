//! Deterministic source-time scene candidates; analysis never mutates a project.
use crate::analysis::{Error as AnalysisError, Result};
use crate::session::wire::{ChangeKind, DetectionResult, SceneCandidate};
use crate::{
    Command, Session,
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState,
        ResourceClass,
    },
};
use media::{SourceAsset, StreamKind, VisionFrame, VisionPrepLimits};
use project::{ItemId, ItemParams};
use serde::{Deserialize, Serialize};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

pub(crate) const RECIPE: &str =
    "scene-v2:rgb-grid32x18:hist16-mad:fade-max2s-latched:hard-priority";
const MAX_PIXELS: u64 = 1 << 24;
const MAX_MARKERS: usize = 4095;
const MAX_BYTES: usize = 1 << 20;
const MAX_FADE: Tick = Tick(2 * TICKS_PER_SECOND);

/// One discovery implementation, with the existing shot-analysis RGB-only
/// hard-cut policy kept distinct from review candidates' histogram/fade policy.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Discovery {
    Reviewed,
    ShotCuts,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DetectionSettings {
    /// Normalized RGB mean absolute difference, 0.01..=1; histogram gate uses the same threshold.
    pub hard_threshold: f32,
    /// Total RGB change across a gradual transition, 0.01..=1.
    pub fade_threshold: f32,
    /// Minimum adjacent RGB change to continue a fade, 0.001..=0.1.
    pub fade_step: f32,
    /// Minimum source-time distance from endpoints and the previous marker, 0..=10 s.
    pub min_scene: Tick,
}
impl Default for DetectionSettings {
    fn default() -> Self {
        Self {
            hard_threshold: 0.28,
            fade_threshold: 0.20,
            fade_step: 0.005,
            min_scene: Tick(TICKS_PER_SECOND / 2),
        }
    }
}
#[derive(Clone)]
pub struct DetectionRequest {
    asset: SourceAsset,
    range: SourceRange,
    settings: DetectionSettings,
    key: CacheKey,
}
pub struct DetectionJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: DetectionRequest,
}
impl DetectionRequest {
    /// Admits <=one day, <=16 Mi-pixel video and <=4095 markers. Decode streams
    /// in <=300 s slices with <=100,000 decoded frames per slice including preroll.
    pub fn new(
        asset: SourceAsset,
        range: SourceRange,
        settings: DetectionSettings,
    ) -> Result<Self> {
        if range.start() < Tick::ZERO
            || range.duration().0 > 86400 * TICKS_PER_SECOND
            || asset.probe().duration.is_some_and(|d| range.end() > d)
            || !settings.hard_threshold.is_finite()
            || !(0.01..=1.).contains(&settings.hard_threshold)
            || !settings.fade_threshold.is_finite()
            || !(0.01..=1.).contains(&settings.fade_threshold)
            || !settings.fade_step.is_finite()
            || !(0.001..=0.1).contains(&settings.fade_step)
            || settings.fade_step >= settings.hard_threshold
            || !(0..=10 * TICKS_PER_SECOND).contains(&settings.min_scene.0)
        {
            return Err(AnalysisError::Edit(crate::session::EditError::Invalid(
                "scene detection settings or range",
            )));
        }
        let video = asset
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "scene detection requires video",
            )))?;
        if video.video.as_ref().is_none_or(|v| {
            v.width == 0 || v.height == 0 || u64::from(v.width) * u64::from(v.height) > MAX_PIXELS
        }) || video
            .duration
            .and_then(|d| video.start.unwrap_or(Tick::ZERO).checked_add(d))
            .is_some_and(|end| range.end() > end)
        {
            return Err(AnalysisError::Edit(crate::session::EditError::Invalid(
                "scene detection video bounds",
            )));
        }
        let canonical = serde_json::to_vec(&(range, settings)).map_err(|_| {
            AnalysisError::Edit(crate::session::EditError::Invalid("scene settings"))
        })?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::SceneDetection,
            source: asset.content_hash().0,
            provider: "local",
            model: "histogram",
            version: RECIPE,
            settings: &canonical,
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self {
            asset,
            range,
            settings,
            key,
        })
    }
    pub fn cache_key(&self) -> CacheKey {
        self.key
    }
    /// Cache hits do not open the source. One ordinary CPU job owns cancellation,
    /// resource admission and output retention; only finish publishes a complete cache entry.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<DetectionJob> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => {
                self.decode(lease.bytes())?;
                Some(lease.bytes().to_vec())
            }
            Ok(None) | Err(crate::RuntimeError::CacheCorrupt) => None,
            Err(e) => return Err(e.into()),
        };
        let request = self.clone();
        let (id, cancel) = runner.submit(
            JobSpec {
                kind: JobKind::DetectScenes,
                class: ResourceClass::Cpu,
                input: self.key,
                bytes: JobBytes {
                    decoded_frames: 3 * MAX_PIXELS * 4,
                    compositor: 16 << 20,
                    result: MAX_BYTES as u64,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                if let Some(bytes) = &cached {
                    return Ok(bytes.clone());
                }
                let result = request.analyze(ctx)?;
                let bytes = serde_json::to_vec(&result)
                    .map_err(|_| AnalysisError::from(crate::session::wire::ControlError::Limit))?;
                if bytes.len() > MAX_BYTES {
                    return Err(AnalysisError::from(
                        crate::session::wire::ControlError::Limit,
                    ));
                }
                Ok(bytes)
            },
        )?;
        Ok(DetectionJob {
            id,
            cancel,
            request: self.clone(),
        })
    }
    fn decode(&self, bytes: &[u8]) -> crate::runtime::Result<DetectionResult> {
        let result = decode(bytes).map_err(|_| crate::RuntimeError::CacheCorrupt)?;
        if result.key != self.key.bytes()
            || result.source != self.asset.content_hash().0
            || result.range != self.range
        {
            return Err(crate::RuntimeError::CacheCorrupt);
        }
        let mut previous = self.range.start();
        for c in &result.candidates {
            if c.at - previous < self.settings.min_scene
                || self.range.end() - c.at < self.settings.min_scene
            {
                return Err(crate::RuntimeError::CacheCorrupt);
            }
            previous = c.at;
        }
        Ok(result)
    }
    pub(crate) fn analyze(&self, ctx: &JobContext) -> Result<DetectionResult> {
        self.analyze_with_mode(ctx, Discovery::Reviewed)
    }
    pub(crate) fn hard_cuts(&self, ctx: &JobContext) -> Result<Vec<Tick>> {
        Ok(self
            .analyze_with_mode(ctx, Discovery::ShotCuts)?
            .candidates
            .into_iter()
            .map(|c| c.at)
            .collect())
    }
    fn analyze_with_mode(&self, ctx: &JobContext, mode: Discovery) -> Result<DetectionResult> {
        let mut detector = Detector::new(self.range, self.settings, mode);
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
                    detector.frame(&frame)?;
                    ctx.progress(
                        JobStage::Analyzing,
                        (frame
                            .source_pts()
                            .saturating_sub(self.range.start())
                            .0
                            .max(0) as f64
                            / self.range.duration().0 as f64) as f32,
                    )
                    .map_err(|_| media::MediaError::Cancelled)
                },
            )?;
            from = end;
        }
        detector.finish_fade()?;
        if detector.count == 0 {
            return Err(AnalysisError::Edit(crate::session::EditError::Unsupported(
                "scene detection has no frames",
            )));
        }
        Ok(DetectionResult {
            key: self.key.bytes(),
            source: self.asset.content_hash().0,
            range: self.range,
            candidates: detector.markers,
            decoded_frames: detector.count,
        })
    }
}
/// Decode and validate retained job bytes: bounded, strictly ordered candidates
/// inside the detected range with finite confidences.
pub fn decode(bytes: &[u8]) -> crate::session::Result<DetectionResult> {
    if bytes.len() > MAX_BYTES {
        return Err(crate::session::EditError::Invalid("scene detection result"));
    }
    let result: DetectionResult = serde_json::from_slice(bytes)
        .map_err(|_| crate::session::EditError::Invalid("scene detection result"))?;
    if result.candidates.len() > MAX_MARKERS || result.decoded_frames == 0 {
        return Err(crate::session::EditError::Invalid("scene detection result"));
    }
    let mut previous = result.range.start();
    for c in &result.candidates {
        if c.at <= previous
            || c.at >= result.range.end()
            || !c.confidence.is_finite()
            || !(0.0..=1.).contains(&c.confidence)
        {
            return Err(crate::session::EditError::Invalid("scene detection result"));
        }
        previous = c.at;
    }
    Ok(result)
}
impl DetectionJob {
    /// Validate the retained output against its request and publish the cache entry once.
    /// The runner keeps the output, so `job_status` can return it until it is evicted.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<DetectionResult>> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => Err(AnalysisError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(AnalysisError::Runtime(crate::RuntimeError::Failed(
                snapshot.error.ok_or(AnalysisError::from(
                    crate::session::SessionError::StaleResult,
                ))?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(AnalysisError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let result = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
}
/// Apply only explicitly reviewed candidate indexes to this source's video item.
/// Refuses duplicates, stale source/range and invalid selections atomically. Source
/// ticks use project's retime mapping. Descending splits retain the original ID;
/// existing batches in one transaction supply one undo step. Empty review is a no-op.
pub fn apply_reviewed(
    session: &mut Session,
    item: ItemId,
    result: &DetectionResult,
    selected: &[usize],
) -> crate::session::Result<()> {
    if selected.len() > MAX_MARKERS {
        return Err(crate::session::EditError::from(
            crate::session::wire::ControlError::Limit,
        ));
    }
    let data = session.project().data();
    let target = data
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .ok_or(crate::session::EditError::NoItem(item))?;
    let source = target
        .source_asset
        .and_then(|id| data.assets.iter().find(|a| a.id() == id))
        .ok_or(crate::session::EditError::from(
            crate::session::SessionError::StaleResult,
        ))?;
    if !matches!(target.content, ItemParams::Video(_)) || source.content_hash().0 != result.source {
        return Err(crate::session::EditError::from(
            crate::session::SessionError::StaleResult,
        ));
    }
    let mut times = std::collections::BTreeSet::new();
    for &index in selected {
        let marker = result
            .candidates
            .get(index)
            .ok_or(crate::session::EditError::Invalid("reviewed scene marker"))?;
        if marker.at <= target.source_range.start()
            || marker.at >= target.source_range.end()
            || marker.at <= result.range.start()
            || marker.at >= result.range.end()
            || !times.insert(target.output_time_of(marker.at)?)
        {
            return Err(crate::session::EditError::Invalid("reviewed scene marker"));
        }
    }
    if times.is_empty() {
        return Ok(());
    }
    let commands: Vec<_> = times
        .into_iter()
        .rev()
        .map(|at| Command::SplitAt { item, at })
        .collect();
    session.transaction(|session| {
        for chunk in commands.chunks(crate::session::wire::MAX_BATCH_COMMANDS - 1) {
            session.apply(Command::Batch {
                commands: chunk.to_vec(),
            })?;
        }
        Ok::<(), crate::EditError>(())
    })?;
    Ok(())
}

// Fixed-size signatures and fade state: no frame history or per-frame timeline.
#[derive(Clone)]
struct Signature {
    rgb: [u8; 32 * 18 * 3],
    histogram: [u16; 48],
}
impl Signature {
    fn new(frame: &VisionFrame) -> media::Result<Self> {
        let mut out = Self {
            rgb: [0; 32 * 18 * 3],
            histogram: [0; 48],
        };
        for y in 0..18usize {
            for x in 0..32usize {
                let sx = ((2 * x + 1) as u64 * u64::from(frame.width()) / 64)
                    .min(u64::from(frame.width()) - 1) as usize;
                let sy = ((2 * y + 1) as u64 * u64::from(frame.height()) / 36)
                    .min(u64::from(frame.height()) - 1) as usize;
                for c in 0..3usize {
                    let v = *frame
                        .rgba()
                        .get((sy * frame.width() as usize + sx) * 4 + c)
                        .ok_or(media::MediaError::Invalid("scene pixel"))?;
                    *out.rgb
                        .get_mut((y * 32 + x) * 3 + c)
                        .ok_or(media::MediaError::Invalid("scene grid"))? = v;
                    let bin = out
                        .histogram
                        .get_mut(c * 16 + usize::from(v / 16))
                        .ok_or(media::MediaError::Invalid("scene histogram"))?;
                    *bin += 1;
                }
            }
        }
        Ok(out)
    }
    fn delta(&self, other: &Self) -> f32 {
        self.rgb
            .iter()
            .zip(&other.rgb)
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum::<u32>() as f32
            / (255. * self.rgb.len() as f32)
    }
    // Preserve the original shot detector's per-channel normalization and f32
    // accumulation order, including its threshold-edge rounding decisions.
    fn shot_delta(&self, other: &Self) -> f32 {
        self.rgb
            .iter()
            .zip(&other.rgb)
            .map(|(a, b)| (f32::from(*a) / 255. - f32::from(*b) / 255.).abs())
            .sum::<f32>()
            / self.rgb.len() as f32
    }
    fn histogram_delta(&self, other: &Self) -> f32 {
        self.histogram
            .iter()
            .zip(&other.histogram)
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum::<u32>() as f32
            / (2. * 3. * 576.)
    }
}
struct Fade {
    anchor: Signature,
    start: Tick,
    end: Tick,
    step: Tick,
    change: f32,
    valid: bool,
}
struct Detector {
    range: SourceRange,
    settings: DetectionSettings,
    mode: Discovery,
    previous: Option<(Tick, Signature)>,
    fade: Option<Fade>,
    markers: Vec<SceneCandidate>,
    count: u64,
}
impl Detector {
    fn new(range: SourceRange, settings: DetectionSettings, mode: Discovery) -> Self {
        Self {
            range,
            settings,
            mode,
            previous: None,
            fade: None,
            markers: Vec::new(),
            count: 0,
        }
    }
    fn emit(&mut self, at: Tick, confidence: f32, kind: ChangeKind) -> media::Result<()> {
        // A reviewed fade must not suppress a later real hard cut. Endpoint
        // admission is checked first so an inadmissible cut cannot erase a fade.
        if kind == ChangeKind::HardCut
            && at > self.range.start()
            && at < self.range.end()
            && at - self.range.start() >= self.settings.min_scene
            && self.range.end() - at >= self.settings.min_scene
            && self
                .markers
                .last()
                .is_some_and(|c| c.kind == ChangeKind::Fade && at - c.at < self.settings.min_scene)
        {
            self.markers.pop();
        }
        let previous = self.markers.last().map_or(self.range.start(), |c| c.at);
        if at <= previous
            || at >= self.range.end()
            || at - previous < self.settings.min_scene
            || self.range.end() - at < self.settings.min_scene
        {
            return Ok(());
        }
        if self.markers.len() == MAX_MARKERS {
            return Err(media::MediaError::TooLarge("scene markers"));
        }
        self.markers.push(SceneCandidate {
            at,
            confidence,
            kind,
        });
        Ok(())
    }
    fn finish_fade(&mut self) -> media::Result<()> {
        if let Some(fade) = self.fade.take()
            && fade.valid
            && fade.change >= self.settings.fade_threshold
        {
            // A fade requires a regular observed frame grid. Irregular/VFR runs
            // are conservatively discarded instead of inventing a midpoint PTS.
            let steps = (fade.end.0 - fade.start.0) / fade.step.0;
            self.emit(
                Tick(fade.start.0 + steps / 2 * fade.step.0),
                fade.change,
                ChangeKind::Fade,
            )?;
        }
        Ok(())
    }
    fn frame(&mut self, frame: &VisionFrame) -> media::Result<()> {
        let at = frame.source_pts().max(self.range.start());
        if self.previous.as_ref().is_some_and(|(pts, _)| at <= *pts) {
            return Ok(());
        }
        let signature = Signature::new(frame)?;
        if let Some((previous_at, previous)) = &self.previous {
            let delta = if self.mode == Discovery::ShotCuts {
                signature.shot_delta(previous)
            } else {
                signature.delta(previous)
            };
            let step = at - *previous_at;
            if delta >= self.settings.hard_threshold
                && (self.mode == Discovery::ShotCuts
                    || signature.histogram_delta(previous) >= self.settings.hard_threshold)
            {
                self.finish_fade()?;
                self.emit(at, delta, ChangeKind::HardCut)?;
            } else if self.mode == Discovery::Reviewed
                && delta >= self.settings.fade_step
                && delta < self.settings.hard_threshold
            {
                let fade = self.fade.get_or_insert_with(|| Fade {
                    anchor: previous.clone(),
                    start: *previous_at,
                    end: at,
                    step,
                    change: 0.,
                    valid: true,
                });
                // Reject the entire continuous run. Starting a fresh anchor
                // every two seconds would merely turn motion into many fades.
                fade.valid &= step == fade.step && at - fade.start <= MAX_FADE;
                if fade.valid {
                    fade.end = at;
                    fade.change = signature.delta(&fade.anchor);
                }
            } else {
                self.finish_fade()?;
            }
        }
        self.previous = Some((at, signature));
        self.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "synthetic PTS/colour fixtures")]
    use super::*;
    fn ms(n: i64) -> Tick {
        Tick(n * (TICKS_PER_SECOND / 1000))
    }
    fn candidates(samples: &[(i64, u8)], end: i64, spacing: i64) -> Vec<SceneCandidate> {
        let settings = DetectionSettings {
            min_scene: ms(spacing),
            ..Default::default()
        };
        let mut detector = Detector::new(
            SourceRange::new(Tick::ZERO, ms(end)).unwrap(),
            settings,
            Discovery::Reviewed,
        );
        for &(at, value) in samples {
            detector
                .frame(
                    &VisionFrame::from_rgba8(ms(at), 1, 1, vec![value, value, value, 255]).unwrap(),
                )
                .unwrap();
        }
        detector.finish_fade().unwrap();
        detector.markers
    }
    #[test]
    fn endpoint_spacing_and_zero_spacing_keep_exact_frame_boundaries() {
        let samples = [(0, 0), (100, 255), (800, 255), (900, 0)];
        assert!(candidates(&samples, 1000, 500).is_empty());
        assert_eq!(
            candidates(&samples, 1000, 0)
                .iter()
                .map(|c| c.at)
                .collect::<Vec<_>>(),
            vec![ms(100), ms(900)]
        );
    }
    #[test]
    fn irregular_fade_run_is_discarded_without_restarting_its_anchor() {
        let samples = [
            (0, 0),
            (100, 30),
            (200, 60),
            (350, 90),
            (450, 120),
            (550, 150),
            (650, 180),
            (750, 180),
        ];
        assert!(candidates(&samples, 1000, 0).is_empty());
    }
    #[test]
    fn adjacent_hard_cut_has_priority_over_a_close_fade() {
        let samples = [
            (0, 0),
            (400, 0),
            (500, 0),
            (600, 42),
            (700, 85),
            (800, 127),
            (900, 170),
            (1000, 212),
            (1100, 255),
            (1200, 0),
            (1800, 0),
            (1900, 0),
        ];
        let result = candidates(&samples, 2000, 500);
        assert_eq!(
            result.iter().map(|c| (c.at, c.kind)).collect::<Vec<_>>(),
            vec![(ms(1200), ChangeKind::HardCut)]
        );
    }
    #[test]
    fn hard_cut_interrupting_an_unfinished_fade_keeps_its_frame() {
        let samples = [
            (0, 0),
            (500, 0),
            (600, 20),
            (700, 40),
            (800, 255),
            (1500, 255),
        ];
        let result = candidates(&samples, 2000, 500);
        assert_eq!(
            result.iter().map(|c| (c.at, c.kind)).collect::<Vec<_>>(),
            vec![(ms(800), ChangeKind::HardCut)]
        );
    }
    #[test]
    fn fade_duration_bound_includes_two_seconds_and_rejects_a_longer_run() {
        for steps in [20, 21] {
            let mut samples: Vec<_> = (0..=steps)
                .map(|n| (n * 100, (n * 255 / steps) as u8))
                .collect();
            samples.push(((steps + 1) * 100, 255));
            let result = candidates(&samples, 3500, 0);
            if steps == 20 {
                assert_eq!(
                    result.iter().map(|c| (c.at, c.kind)).collect::<Vec<_>>(),
                    vec![(ms(1000), ChangeKind::Fade)]
                );
            } else {
                assert!(result.is_empty());
            }
        }
    }
    #[test]
    fn rgb_only_cut_policy_keeps_original_threshold_rounding() {
        let settings = DetectionSettings {
            hard_threshold: 72. / 255.,
            min_scene: Tick::ZERO,
            ..Default::default()
        };
        for mode in [Discovery::Reviewed, Discovery::ShotCuts] {
            let mut detector = Detector::new(
                SourceRange::new(Tick::ZERO, ms(2000)).unwrap(),
                settings,
                mode,
            );
            for (at, value) in [(0, 0), (1000, 72)] {
                detector
                    .frame(
                        &VisionFrame::from_rgba8(ms(at), 1, 1, vec![value, value, value, 255])
                            .unwrap(),
                    )
                    .unwrap();
            }
            // Old per-channel float accumulation yields 0.28235018, below
            // threshold 0.28235295. Reviewed integer-grid scoring reaches it.
            assert_eq!(
                detector.markers.len(),
                usize::from(mode == Discovery::Reviewed)
            );
        }
    }
}

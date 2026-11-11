//! Classifies source speech audio into speech and pause spans by frame energy.
use crate::{
    EditError,
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState,
        ResourceClass,
    },
};
use media::{SPEECH_SAMPLE_RATE, SourceAsset, SpeechExtractSettings, StreamKind};
use serde::{Deserialize, Serialize};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

/// 20 ms at the speech extract's 16 kHz mono rate.
const FRAME_SAMPLES: usize = SPEECH_SAMPLE_RATE as usize / 50;
/// One frame is 20 ms, so Martin's 1.5 s minimum-search window is 75 frames.
const NOISE_BLOCK_FRAMES: usize = 75;
/// Low-percentile noise floor and margin, from the percentile validator.
const NOISE_PERCENTILE: f64 = 0.10;
const NOISE_MARGIN_DB: f64 = 6.0;
/// The gate also stays this far below the median frame, twice the margin, so a
/// unimodal speech signal cannot be swallowed by its own floor estimate.
const NOISE_GUARD_DB: f64 = 2.0 * NOISE_MARGIN_DB;
const MAX_SECONDS: i64 = 86_400;
/// 20 ms frames over one day, plus the partial final frame.
const MAX_FRAMES: usize = MAX_SECONDS as usize * 50 + 1;
const MAX_SPANS: usize = 288_000;
const MAX_RUNS: usize = 2 * MAX_SPANS;
const MAX_BYTES: usize = 32 << 20;
const RECIPE: &str =
    "speech-activity-v2:rms-20ms:minstats-1.5s-p10:margin.6db:min-speech.1s:min-pause.5s";

/// Run lengths. Defaults cite their source in the analysis README; admitted
/// ranges only keep hostile settings bounded.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechActivitySettings {
    /// Shortest silence between speech that is a pause: 0..=10 s. Default 0.5 s.
    pub min_pause: Tick,
    /// Shortest speech between pauses that stays speech: 0..=10 s. Default 0.1 s.
    pub min_speech: Tick,
}
impl Default for SpeechActivitySettings {
    fn default() -> Self {
        Self {
            min_pause: Tick(TICKS_PER_SECOND / 2),
            min_speech: Tick(TICKS_PER_SECOND / 10),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Speech,
    Pause,
}

/// One contiguous source-time interval whose frames share an activity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechActivitySpan {
    pub range: SourceRange,
    pub activity: Activity,
    /// Mean frame margin normalized over the gate-to-full-scale span; a heuristic
    /// score, not a calibrated probability.
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechActivityResult {
    version: String,
    key: [u8; 32],
    source: [u8; 32],
    range: SourceRange,
    pub analyzed_frames: u64,
    pub spans: Vec<SpeechActivitySpan>,
}

/// One source range's energy classification, cached by source content hash.
#[derive(Clone)]
pub struct SpeechActivityRequest {
    asset: SourceAsset,
    range: SourceRange,
    settings: SpeechActivitySettings,
    key: CacheKey,
}

/// Retained runner job. `finish` caches only a complete, validated output; it is
/// retryable after cache pressure and returns None while work is outstanding.
pub struct SpeechActivityJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: SpeechActivityRequest,
}

impl SpeechActivityRequest {
    /// Admit at most one day, an audio stream covering the range and bounded
    /// settings. The first frame starts at the range start and the last ends at
    /// its end, so spans always cover the requested source time.
    pub fn new(
        asset: SourceAsset,
        range: SourceRange,
        settings: SpeechActivitySettings,
    ) -> Result<Self, EditError> {
        if range.start() < Tick::ZERO
            || range.duration().0 > MAX_SECONDS * TICKS_PER_SECOND
            || asset.probe().duration.is_some_and(|end| range.end() > end)
            || !(0..=10 * TICKS_PER_SECOND).contains(&settings.min_pause.0)
            || !(0..=10 * TICKS_PER_SECOND).contains(&settings.min_speech.0)
        {
            return Err(EditError::Invalid("speech activity request"));
        }
        let audio = asset
            .probe()
            .streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Audio)
            .ok_or(EditError::Unsupported("speech activity requires audio"))?;
        if audio.audio.is_none()
            || audio
                .duration
                .and_then(|duration| audio.start.unwrap_or(Tick::ZERO).checked_add(duration))
                .is_some_and(|end| range.end() > end)
        {
            return Err(EditError::Invalid("speech activity audio bounds"));
        }
        let canonical = serde_json::to_vec(&(range, settings))
            .map_err(|_| EditError::Invalid("speech activity settings"))?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::SpeechActivity,
            source: asset.content_hash().0,
            provider: "local",
            model: "energy",
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

    /// Enqueue one CPU job. A warm cache entry opens no source. Only `finish`
    /// publishes a complete cache entry.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<SpeechActivityJob, EditError> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => {
                self.decode(lease.bytes())?;
                Some(lease.bytes().to_vec())
            }
            Ok(None) | Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt)) => None,
            Err(error) => return Err(error),
        };
        let request = self.clone();
        // The media speech extract retains one default 10-minute PCM window
        // (600 s * 16 kHz * 4 bytes = 38.4 MB). The frame history is one f32
        // per 20 ms (17.3 MB/day); block minima, runs and spans share 96 MiB.
        let spec = JobSpec {
            kind: JobKind::AnalyzeSpeechActivity,
            class: ResourceClass::Cpu,
            input: self.key,
            bytes: JobBytes {
                decoded_frames: 40 << 20,
                compositor: 96 << 20,
                result: MAX_BYTES as u64,
                ..Default::default()
            },
        };
        let (id, cancel) = runner.submit(spec, move |context| {
            context.check_cancelled()?;
            if let Some(bytes) = &cached {
                return Ok(bytes.clone());
            }
            let result = request.analyze(context)?;
            context.check_cancelled()?;
            let bytes = serde_json::to_vec(&result).map_err(|_| EditError::Limit)?;
            if bytes.len() > MAX_BYTES {
                return Err(EditError::Limit);
            }
            Ok(bytes)
        })?;
        Ok(SpeechActivityJob {
            id,
            cancel,
            request: self.clone(),
        })
    }

    fn decode(&self, bytes: &[u8]) -> Result<SpeechActivityResult, EditError> {
        if bytes.len() > MAX_BYTES {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let result: SpeechActivityResult = serde_json::from_slice(bytes)
            .map_err(|_| EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
        if result.version != RECIPE
            || result.key != self.key.bytes()
            || result.source != self.asset.content_hash().0
            || result.range != self.range
            || result.analyzed_frames == 0
            || result.spans.is_empty()
            || result.spans.len() > MAX_SPANS
        {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let mut start = self.range.start();
        let mut previous = None;
        for span in &result.spans {
            if span.range.start() != start
                || span.range.duration().0 <= 0
                || !span.confidence.is_finite()
                || !(0.0..=1.0).contains(&span.confidence)
                || previous == Some(span.activity)
            {
                return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
            }
            start = span.range.end();
            previous = Some(span.activity);
        }
        if start != self.range.end() {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        Ok(result)
    }

    /// Decode and classify this request inside a job without touching the artifact
    /// cache. Progress is reported inside `base..=base+span`, so a caller scanning
    /// several ranges keeps one job progress scale.
    pub(crate) fn detect(
        &self,
        context: &JobContext,
        base: f32,
        span: f32,
    ) -> Result<(Vec<SpeechActivitySpan>, u64), EditError> {
        if !base.is_finite() || !span.is_finite() || base < 0.0 || span < 0.0 || base + span > 1.0 {
            return Err(EditError::Invalid("speech activity progress"));
        }
        let extract = SpeechExtractSettings {
            overlap: Tick::ZERO,
            ..SpeechExtractSettings::default()
        };
        let mut detector = Detector::new(self.range, self.settings);
        let mut failure = None;
        let decoded = media::extract_speech_windows_with_cancel(
            &self.asset,
            self.range,
            &extract,
            context.media_cancel_flag(),
            |window| {
                context
                    .check_cancelled()
                    .map_err(|_| media::MediaError::Cancelled)?;
                if let Err(error) = detector.push_samples(&window.samples, window.source_range) {
                    failure = Some(error);
                    return Err(media::MediaError::Invalid("speech activity window"));
                }
                let progress = ((window.source_range.end().0 - self.range.start().0) as f64
                    / self.range.duration().0 as f64)
                    .clamp(0.0, 1.0);
                context
                    .progress(JobStage::Analyzing, base + span * progress as f32)
                    .map_err(|_| media::MediaError::Cancelled)
            },
        );
        if let Some(error) = failure {
            return Err(error);
        }
        decoded?;
        context.check_cancelled()?;
        detector.finish()
    }

    fn analyze(&self, context: &JobContext) -> Result<SpeechActivityResult, EditError> {
        let (spans, analyzed_frames) = self.detect(context, 0.0, 1.0)?;
        Ok(SpeechActivityResult {
            version: RECIPE.into(),
            key: self.key.bytes(),
            source: self.asset.content_hash().0,
            range: self.range,
            analyzed_frames,
            spans,
        })
    }
}

impl SpeechActivityJob {
    /// Validate and cache the finished result; typed terminal failures and
    /// cancellation propagate.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<SpeechActivityResult>, EditError> {
        let snapshot = runner.snapshot(self.id).ok_or(EditError::JobNotRetained)?;
        match snapshot.state {
            JobState::Cancelled => Err(EditError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(EditError::Runtime(crate::RuntimeError::Failed(
                snapshot.error.ok_or(EditError::StaleResult)?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                let result = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::JobNotRetained)?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Clone)]
struct Run {
    activity: Activity,
    start: Tick,
    end: Tick,
    frames: u64,
    speech_confidence: f64,
    pause_confidence: f64,
}
impl Run {
    fn duration(&self) -> i64 {
        self.end.0.saturating_sub(self.start.0)
    }
    fn with_activity(mut self, activity: Activity) -> Self {
        self.activity = activity;
        self
    }
}

struct Detector {
    range: SourceRange,
    min_pause: i64,
    min_speech: i64,
    next_window: Option<Tick>,
    pending_start: Option<Tick>,
    pending: Vec<f32>,
    first_start: Option<Tick>,
    frames_db: Vec<f32>,
    histogram: [u32; 121],
    block_min: Option<f32>,
    block_frames: usize,
    block_minima: Vec<f32>,
}
impl Detector {
    fn new(range: SourceRange, settings: SpeechActivitySettings) -> Self {
        Self {
            range,
            min_pause: settings.min_pause.0,
            min_speech: settings.min_speech.0,
            next_window: None,
            pending_start: None,
            pending: Vec::new(),
            first_start: None,
            frames_db: Vec::new(),
            histogram: [0; 121],
            block_min: None,
            block_frames: 0,
            block_minima: Vec::new(),
        }
    }

    /// Append one contiguous PCM window and record every complete frame.
    fn push_samples(&mut self, samples: &[f32], window: SourceRange) -> Result<(), EditError> {
        match self.next_window {
            Some(expected) if window.start() != expected => {
                return Err(EditError::Invalid("speech activity window clock"));
            }
            None if window.start() < self.range.start()
                || window.start().0 - self.range.start().0 > ticks_of(1)?.0 =>
            {
                return Err(EditError::Invalid("speech activity first window"));
            }
            _ => {}
        }
        self.next_window = Some(window.end());
        if samples.is_empty() {
            return Ok(());
        }
        let expected = match self.pending_start {
            Some(start) => start
                .checked_add(ticks_of(self.pending.len())?)
                .ok_or(EditError::Limit)?,
            None => window.start(),
        };
        if expected != window.start() {
            return Err(EditError::Invalid("speech activity sample clock"));
        }
        self.pending_start.get_or_insert(window.start());
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= FRAME_SAMPLES {
            self.push_frame()?;
        }
        Ok(())
    }

    fn push_frame(&mut self) -> Result<(), EditError> {
        let samples = self
            .pending
            .get(..FRAME_SAMPLES)
            .ok_or(EditError::Invalid("speech activity frame"))?;
        let db = frame_db(samples);
        let start = self
            .pending_start
            .ok_or(EditError::Invalid("speech activity frame clock"))?;
        let end = start
            .checked_add(ticks_of(FRAME_SAMPLES)?)
            .ok_or(EditError::Limit)?;
        self.record(db, start)?;
        self.pending.drain(..FRAME_SAMPLES);
        self.pending_start = Some(end);
        Ok(())
    }

    fn record(&mut self, db: f64, start: Tick) -> Result<(), EditError> {
        if !db.is_finite() && db != f64::NEG_INFINITY {
            return Err(EditError::Invalid("speech activity energy"));
        }
        if self.frames_db.len() >= MAX_FRAMES {
            return Err(EditError::Limit);
        }
        self.first_start.get_or_insert(start);
        let value = db as f32;
        self.frames_db.push(value);
        let bin = if db.is_infinite() {
            0
        } else {
            ((db.clamp(-120.0, 0.0) + 120.0).round() as usize).min(120)
        };
        let count = self
            .histogram
            .get_mut(bin)
            .ok_or(EditError::Invalid("speech activity histogram"))?;
        *count = count.checked_add(1).ok_or(EditError::Limit)?;
        self.block_min = Some(self.block_min.map_or(value, |min| min.min(value)));
        self.block_frames += 1;
        if self.block_frames == NOISE_BLOCK_FRAMES {
            self.block_minima.push(
                self.block_min
                    .take()
                    .ok_or(EditError::Invalid("speech activity block"))?,
            );
            self.block_frames = 0;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(Vec<SpeechActivitySpan>, u64), EditError> {
        let partial = self.pending.len();
        if partial > 0 {
            let samples = self
                .pending
                .get(..)
                .ok_or(EditError::Invalid("speech activity frame"))?;
            let db = frame_db(samples);
            let start = self
                .pending_start
                .ok_or(EditError::Invalid("speech activity frame clock"))?;
            self.record(db, start)?;
        }
        if let Some(min) = self.block_min.take() {
            self.block_minima.push(min);
        }
        let frames = self.frames_db.len();
        if frames == 0 {
            return Err(EditError::Unsupported("speech activity has no samples"));
        }
        let floor = percentile_sorted(&mut self.block_minima, NOISE_PERCENTILE)?;
        let median = histogram_percentile(&self.histogram, 0.5)?;
        // Minimum statistics gives the floor; the low-percentile selection and
        // 6 dB margin are the percentile validator. The guard keeps the gate
        // below the median so continuous speech cannot become its own floor.
        let gate = (floor + NOISE_MARGIN_DB).min(median - NOISE_GUARD_DB);
        let first = self
            .first_start
            .ok_or(EditError::Invalid("speech activity frame clock"))?;
        let step = ticks_of(FRAME_SAMPLES)?;
        let mut runs = Vec::new();
        for (index, db) in self.frames_db.iter().enumerate() {
            let start = frame_start(first, step, index)?;
            let samples = if index + 1 == frames && partial > 0 {
                partial
            } else {
                FRAME_SAMPLES
            };
            let end = start
                .checked_add(ticks_of(samples)?)
                .ok_or(EditError::Limit)?;
            let (activity, speech_confidence, pause_confidence) = classify_db(f64::from(*db), gate);
            merge_run(
                &mut runs,
                Run {
                    activity,
                    start,
                    end,
                    frames: 1,
                    speech_confidence,
                    pause_confidence,
                },
            )?;
        }
        let mut spans = finalize(runs, self.min_speech, self.min_pause)?;
        let last = spans
            .last_mut()
            .ok_or(EditError::Unsupported("speech activity has no spans"))?;
        if last.range.end() > self.range.end() {
            last.range = SourceRange::new(last.range.start(), self.range.end())?;
        } else if last.range.end() < self.range.end() {
            // Resampling can leave less than one sample of rounding at the edge.
            if self.range.end().0 - last.range.end().0 > ticks_of(1)?.0 {
                return Err(EditError::Media(media::MediaError::Invalid(
                    "speech activity sample clock",
                )));
            }
            last.range = SourceRange::new(last.range.start(), self.range.end())?;
        }
        let first = spans
            .first_mut()
            .ok_or(EditError::Unsupported("speech activity has no spans"))?;
        if first.range.start() > self.range.start() {
            // The first decoded sample is the first sample boundary at or after
            // the requested start; extend the span across that sub-sample gap.
            if first.range.start().0 - self.range.start().0 > ticks_of(1)?.0 {
                return Err(EditError::Invalid("speech activity range"));
            }
            first.range = SourceRange::new(self.range.start(), first.range.end())?;
        } else if first.range.start() != self.range.start() {
            return Err(EditError::Invalid("speech activity range"));
        }
        Ok((spans, frames as u64))
    }
}

fn frame_db(samples: &[f32]) -> f64 {
    let mean = if samples.is_empty() {
        0.0
    } else {
        samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / samples.len() as f64
    };
    let rms = mean.sqrt();
    if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        f64::NEG_INFINITY
    }
}

fn classify_db(db: f64, gate: f64) -> (Activity, f64, f64) {
    if gate == f64::NEG_INFINITY {
        return if db.is_finite() {
            (Activity::Speech, 1.0, 0.0)
        } else {
            (Activity::Pause, 0.0, 1.0)
        };
    }
    let span = (-gate).max(1.0);
    if db > gate {
        let confidence = ((db - gate) / span).clamp(0.0, 1.0);
        (Activity::Speech, confidence, 0.0)
    } else if db.is_finite() {
        let confidence = ((gate - db) / span).clamp(0.0, 1.0);
        (Activity::Pause, 0.0, confidence)
    } else {
        (Activity::Pause, 0.0, 1.0)
    }
}

fn frame_start(first: Tick, step: Tick, index: usize) -> Result<Tick, EditError> {
    let index = i64::try_from(index).map_err(|_| EditError::Limit)?;
    let offset = step.0.checked_mul(index).ok_or(EditError::Limit)?;
    Ok(Tick(first.0.checked_add(offset).ok_or(EditError::Limit)?))
}

fn percentile_sorted(values: &mut [f32], percentile: f64) -> Result<f64, EditError> {
    if values.is_empty() {
        return Err(EditError::Unsupported("speech activity has no blocks"));
    }
    values.sort_unstable_by(f32::total_cmp);
    let rank = (((values.len() - 1) as f64) * percentile).round() as usize;
    Ok(f64::from(
        *values
            .get(rank)
            .ok_or(EditError::Invalid("speech activity percentile"))?,
    ))
}

fn histogram_percentile(histogram: &[u32; 121], percentile: f64) -> Result<f64, EditError> {
    let total = histogram
        .iter()
        .try_fold(0u64, |total, count| total.checked_add(u64::from(*count)))
        .ok_or(EditError::Limit)?;
    if total == 0 {
        return Err(EditError::Unsupported("speech activity has no frames"));
    }
    let rank = (((total - 1) as f64) * percentile).round() as u64;
    let mut cumulative = 0u64;
    for (index, count) in histogram.iter().enumerate() {
        cumulative = cumulative
            .checked_add(u64::from(*count))
            .ok_or(EditError::Limit)?;
        if cumulative > rank {
            return Ok(index as f64 - 120.0);
        }
    }
    Ok(0.0)
}

fn finalize(
    runs: Vec<Run>,
    min_speech: i64,
    min_pause: i64,
) -> Result<Vec<SpeechActivitySpan>, EditError> {
    // Drop short speech first so a click cannot split one long pause into two
    // short ones that hangover would then swallow.
    let mut without_blips = Vec::with_capacity(runs.len());
    let last = runs.len().saturating_sub(1);
    for (index, run) in runs.into_iter().enumerate() {
        let activity = if run.activity == Activity::Speech
            && run.duration() < min_speech
            && index > 0
            && index < last
        {
            Activity::Pause
        } else {
            run.activity
        };
        merge_run(&mut without_blips, run.with_activity(activity))?;
    }
    let mut without_dips = Vec::with_capacity(without_blips.len());
    let last = without_blips.len().saturating_sub(1);
    for (index, run) in without_blips.into_iter().enumerate() {
        let activity = if run.activity == Activity::Pause
            && run.duration() < min_pause
            && index > 0
            && index < last
        {
            Activity::Speech
        } else {
            run.activity
        };
        merge_run(&mut without_dips, run.with_activity(activity))?;
    }
    if without_dips.is_empty() {
        return Err(EditError::Unsupported("speech activity has no spans"));
    }
    if without_dips.len() > MAX_SPANS {
        return Err(EditError::Limit);
    }
    let mut spans = Vec::with_capacity(without_dips.len());
    for run in without_dips {
        let sum = match run.activity {
            Activity::Speech => run.speech_confidence,
            Activity::Pause => run.pause_confidence,
        };
        let confidence = sum / run.frames as f64;
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(EditError::Invalid("speech activity confidence"));
        }
        spans.push(SpeechActivitySpan {
            range: SourceRange::new(run.start, run.end)?,
            activity: run.activity,
            confidence: confidence as f32,
        });
    }
    Ok(spans)
}

fn merge_run(runs: &mut Vec<Run>, run: Run) -> Result<(), EditError> {
    if let Some(last) = runs.last_mut()
        && last.activity == run.activity
    {
        if last.end != run.start {
            return Err(EditError::Invalid("speech activity run clock"));
        }
        last.end = run.end;
        last.frames = last
            .frames
            .checked_add(run.frames)
            .ok_or(EditError::Limit)?;
        last.speech_confidence += run.speech_confidence;
        last.pause_confidence += run.pause_confidence;
        return Ok(());
    }
    if runs.len() >= MAX_RUNS {
        return Err(EditError::Limit);
    }
    runs.push(run);
    Ok(())
}

fn ticks_of(samples: usize) -> Result<Tick, EditError> {
    let samples = i64::try_from(samples).map_err(|_| EditError::Limit)?;
    Ok(Tick::from_units(
        samples,
        i64::from(SPEECH_SAMPLE_RATE),
        Rounding::Nearest,
    )?)
}

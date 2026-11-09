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
const MAX_SECONDS: i64 = 86_400;
const MAX_SPANS: usize = 288_000;
const MAX_RUNS: usize = 2 * MAX_SPANS;
const MAX_BYTES: usize = 32 << 20;
const RECIPE: &str = "speech-activity-v1:rms-20ms:min-speech.1s:min-pause.5s";

/// Frame log-energy gate and run lengths. Defaults cite their source in the
/// analysis README; admitted ranges only keep hostile settings bounded.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechActivitySettings {
    /// Frame RMS at or below this gate is silence, in dBFS: -120..=-20. Default -60.
    pub noise_db: f32,
    /// Shortest silence between speech that is a pause: 0..=10 s. Default 0.5 s.
    pub min_pause: Tick,
    /// Shortest speech between pauses that stays speech: 0..=10 s. Default 0.1 s.
    pub min_speech: Tick,
}
impl Default for SpeechActivitySettings {
    fn default() -> Self {
        Self {
            noise_db: -60.,
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
            || !settings.noise_db.is_finite()
            || !(-120.0..=-20.0).contains(&settings.noise_db)
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
            Ok(None) | Err(EditError::CacheCorrupt) => None,
            Err(error) => return Err(error),
        };
        let request = self.clone();
        // The media speech extract retains one default 10-minute PCM window
        // (600 s * 16 kHz * 4 bytes = 38.4 MB); runs and spans share 64 MiB.
        let spec = JobSpec {
            kind: JobKind::AnalyzeSpeechActivity,
            class: ResourceClass::Cpu,
            input: self.key,
            bytes: JobBytes {
                decoded_frames: 40 << 20,
                compositor: 64 << 20,
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
            return Err(EditError::CacheCorrupt);
        }
        let result: SpeechActivityResult =
            serde_json::from_slice(bytes).map_err(|_| EditError::CacheCorrupt)?;
        if result.version != RECIPE
            || result.key != self.key.bytes()
            || result.source != self.asset.content_hash().0
            || result.range != self.range
            || result.analyzed_frames == 0
            || result.spans.is_empty()
            || result.spans.len() > MAX_SPANS
        {
            return Err(EditError::CacheCorrupt);
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
                return Err(EditError::CacheCorrupt);
            }
            start = span.range.end();
            previous = Some(span.activity);
        }
        if start != self.range.end() {
            return Err(EditError::CacheCorrupt);
        }
        Ok(result)
    }

    fn analyze(&self, context: &JobContext) -> Result<SpeechActivityResult, EditError> {
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
                    .progress(JobStage::Analyzing, progress as f32)
                    .map_err(|_| media::MediaError::Cancelled)
            },
        );
        if let Some(error) = failure {
            return Err(error);
        }
        decoded?;
        context.check_cancelled()?;
        let (spans, analyzed_frames) = detector.finish()?;
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
            JobState::Cancelled => Err(EditError::Cancelled),
            JobState::Failed => Err(EditError::JobFailure(
                snapshot.error.ok_or(EditError::StaleResult)?,
            )),
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
    noise_db: f64,
    min_pause: i64,
    min_speech: i64,
    next_window: Option<Tick>,
    pending_start: Option<Tick>,
    pending: Vec<f32>,
    runs: Vec<Run>,
    frames: u64,
}
impl Detector {
    fn new(range: SourceRange, settings: SpeechActivitySettings) -> Self {
        Self {
            range,
            noise_db: f64::from(settings.noise_db),
            min_pause: settings.min_pause.0,
            min_speech: settings.min_speech.0,
            next_window: None,
            pending_start: None,
            pending: Vec::new(),
            runs: Vec::new(),
            frames: 0,
        }
    }

    /// Append one contiguous PCM window and classify every complete frame.
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
        let (activity, speech_confidence, pause_confidence) = classify(samples, self.noise_db);
        let start = self
            .pending_start
            .ok_or(EditError::Invalid("speech activity frame clock"))?;
        let end = start
            .checked_add(ticks_of(FRAME_SAMPLES)?)
            .ok_or(EditError::Limit)?;
        self.push_run(activity, start, end, speech_confidence, pause_confidence)?;
        self.pending.drain(..FRAME_SAMPLES);
        self.pending_start = Some(end);
        self.frames = self.frames.checked_add(1).ok_or(EditError::Limit)?;
        Ok(())
    }

    fn push_run(
        &mut self,
        activity: Activity,
        start: Tick,
        end: Tick,
        speech_confidence: f64,
        pause_confidence: f64,
    ) -> Result<(), EditError> {
        if end <= start {
            return Err(EditError::Invalid("speech activity run"));
        }
        if let Some(last) = self.runs.last_mut()
            && last.activity == activity
        {
            if last.end != start {
                return Err(EditError::Invalid("speech activity run clock"));
            }
            last.end = end;
            last.frames = last.frames.checked_add(1).ok_or(EditError::Limit)?;
            last.speech_confidence += speech_confidence;
            last.pause_confidence += pause_confidence;
            return Ok(());
        }
        if self.runs.len() >= MAX_RUNS {
            return Err(EditError::Limit);
        }
        self.runs.push(Run {
            activity,
            start,
            end,
            frames: 1,
            speech_confidence,
            pause_confidence,
        });
        Ok(())
    }

    fn finish(mut self) -> Result<(Vec<SpeechActivitySpan>, u64), EditError> {
        if !self.pending.is_empty() {
            let samples = self
                .pending
                .get(..)
                .ok_or(EditError::Invalid("speech activity frame"))?;
            let (activity, speech_confidence, pause_confidence) = classify(samples, self.noise_db);
            let start = self
                .pending_start
                .ok_or(EditError::Invalid("speech activity frame clock"))?;
            let end = start
                .checked_add(ticks_of(samples.len())?)
                .ok_or(EditError::Limit)?;
            self.push_run(activity, start, end, speech_confidence, pause_confidence)?;
            self.frames = self.frames.checked_add(1).ok_or(EditError::Limit)?;
        }
        if self.frames == 0 {
            return Err(EditError::Unsupported("speech activity has no samples"));
        }
        let mut spans = finalize(self.runs, self.min_speech, self.min_pause)?;
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
        Ok((spans, self.frames))
    }
}

fn classify(samples: &[f32], noise_db: f64) -> (Activity, f64, f64) {
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
    let db = if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        f64::NEG_INFINITY
    };
    let span = -noise_db;
    if db > noise_db {
        let confidence = ((db - noise_db) / span).clamp(0.0, 1.0);
        (Activity::Speech, confidence, 0.0)
    } else if db.is_finite() {
        let confidence = ((noise_db - db) / span).clamp(0.0, 1.0);
        (Activity::Pause, 0.0, confidence)
    } else {
        (Activity::Pause, 0.0, 1.0)
    }
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

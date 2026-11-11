//! Generates and caches bounded source peak summaries for clip waveforms.
use crate::{
    EditError,
    cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState,
        ResourceClass,
    },
};
use media::{AudioInfo, PEAK_SUMMARY_MAX_BYTES, PeakSummary, SourceAsset, StreamKind};
use time::{SourceRange, Tick, TimeRange};

/// One display column may not request more bars than this; hostile widths refuse.
pub const MAX_COLUMNS: usize = 4096;
const RECIPE: &str = "waveform-peaks-v1:bucket.128:source-rate-minmax";
const MAX_BYTES: usize = PEAK_SUMMARY_MAX_BYTES;
/// One decoded audio frame converted to planar f32, twice over.
const DECODE_BYTES: u64 = 16 << 20;

/// One source asset's peak summary request, cached by content and audio layout.
#[derive(Clone)]
pub struct WaveformRequest {
    asset: SourceAsset,
    key: CacheKey,
}

/// Retained runner job. `finish` caches only a complete, validated summary; it
/// is retryable after cache pressure and returns None while work is outstanding.
pub struct WaveformJob {
    pub id: JobId,
    pub cancel: CancelToken,
    request: WaveformRequest,
}

impl WaveformRequest {
    /// Admit one source with a mono or stereo audio stream. Keys bind the source
    /// content hash, sample rate, channel count and the versioned recipe.
    pub fn new(asset: SourceAsset) -> Result<Self, EditError> {
        let audio = audio_info(&asset)?;
        if !(8_000..=192_000).contains(&audio.sample_rate) || !(1..=2).contains(&audio.channels) {
            return Err(EditError::Unsupported(
                "waveform requires mono or stereo audio",
            ));
        }
        let mut settings = [0u8; 6];
        settings
            .get_mut(..4)
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?
            .copy_from_slice(&audio.sample_rate.to_le_bytes());
        settings
            .get_mut(4..)
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?
            .copy_from_slice(&audio.channels.to_le_bytes());
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Waveform,
            source: asset.content_hash().0,
            provider: "local",
            model: "peaks",
            version: RECIPE,
            settings: &settings,
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Self { asset, key })
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
    ) -> Result<WaveformJob, EditError> {
        let cached = match cache.get(self.key) {
            Ok(Some(lease)) => self
                .decode(lease.bytes())
                .ok()
                .map(|_| lease.bytes().to_vec()),
            Ok(None) | Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt)) => None,
            Err(error) => return Err(error),
        };
        let request = self.clone();
        let spec = JobSpec {
            kind: JobKind::GenerateWaveform,
            class: ResourceClass::Cpu,
            input: self.key,
            bytes: JobBytes {
                decoded_frames: DECODE_BYTES,
                result: MAX_BYTES as u64,
                ..Default::default()
            },
        };
        let (id, cancel) = runner.submit(spec, move |context| {
            context.check_cancelled()?;
            if let Some(bytes) = &cached {
                return Ok(bytes.clone());
            }
            let summary = request.analyze(context)?;
            context.check_cancelled()?;
            let bytes = summary.encode()?;
            if bytes.len() > MAX_BYTES {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ));
            }
            Ok(bytes)
        })?;
        Ok(WaveformJob {
            id,
            cancel,
            request: self.clone(),
        })
    }

    fn analyze(&self, context: &JobContext) -> Result<PeakSummary, EditError> {
        context.progress(JobStage::Analyzing, 0.0)?;
        let summary = media::peak_summary(&self.asset, context.media_cancel_flag())?;
        context.check_cancelled()?;
        Ok(summary)
    }

    fn decode(&self, bytes: &[u8]) -> Result<PeakSummary, EditError> {
        if bytes.len() > MAX_BYTES {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let summary = PeakSummary::decode(bytes)
            .map_err(|_| EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
        let audio = audio_info(&self.asset)
            .map_err(|_| EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
        if summary.sample_rate() != audio.sample_rate || summary.channels() != audio.channels {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        Ok(summary)
    }
}

impl WaveformJob {
    /// Validate and cache the finished summary; typed terminal failures and
    /// cancellation propagate.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> Result<Option<PeakSummary>, EditError> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => Err(EditError::Runtime(crate::RuntimeError::Cancelled)),
            JobState::Failed => Err(EditError::Runtime(crate::RuntimeError::Failed(
                snapshot
                    .error
                    .ok_or(EditError::Session(crate::SessionError::StaleResult))?,
            ))),
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let summary = self.request.decode(bytes)?;
                cache.put(self.request.key, bytes)?;
                runner
                    .take_output(self.id)
                    .ok_or(EditError::Runtime(crate::RuntimeError::JobNotRetained))?;
                Ok(Some(summary))
            }
            _ => Ok(None),
        }
    }
}

/// One reduced display column. Amplitudes are linear; `over` marks any sample
/// outside -1..=1 so the caller can draw a >0 dBFS tip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveformBar {
    pub min: f32,
    pub max: f32,
    pub over: bool,
}

/// Reduced peaks for a clip's visible output range, one bar per requested column.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveformDisplay {
    bars: Vec<WaveformBar>,
}

impl WaveformDisplay {
    pub fn bars(&self) -> &[WaveformBar] {
        &self.bars
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}

/// Cache-only answer for one clip's visible range. Never decodes media.
#[derive(Debug)]
pub enum WaveformState {
    /// Complete summary and mapped bars for this clip.
    Ready(WaveformDisplay),
    /// Generation has not produced a cached summary yet.
    Pending,
    /// The cached summary cannot be decoded and must be regenerated.
    Failed(EditError),
    /// The clip's source has no mono or stereo audio stream.
    Unsupported,
}

/// Map one clip's visible output range through trim and retime and reduce the
/// cached summary to `columns` bars. Reads only the artifact cache.
///
/// `visible` is clamped to the clip. A range outside the clip, a mismatched
/// asset or more than [`MAX_COLUMNS`] columns is `Invalid`; a missing cache
/// entry is `Pending`; an undecodable one is `Failed`.
pub fn display(
    asset: &SourceAsset,
    item: &project::Item,
    visible: TimeRange,
    columns: usize,
    cache: &mut ArtifactCache,
) -> Result<WaveformState, EditError> {
    if !(1..=MAX_COLUMNS).contains(&columns) {
        return Err(EditError::Invalid("waveform display columns"));
    }
    if item.source_asset != Some(asset.id()) {
        return Err(EditError::Invalid("waveform display asset"));
    }
    let request = match WaveformRequest::new(asset.clone()) {
        Ok(request) => request,
        Err(EditError::Unsupported(_)) => return Ok(WaveformState::Unsupported),
        Err(error) => return Err(error),
    };
    let clip = TimeRange::new(item.start, item.end()?)?;
    let span = visible
        .intersect(clip)
        .ok_or(EditError::Invalid("waveform display outside clip"))?;
    let lease = match cache.get(request.key) {
        Ok(Some(lease)) => lease,
        Ok(None) => return Ok(WaveformState::Pending),
        Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt)) => {
            return Ok(WaveformState::Failed(EditError::Runtime(
                crate::RuntimeError::CacheCorrupt,
            )));
        }
        Err(error) => return Err(error),
    };
    let summary = match request.decode(lease.bytes()) {
        Ok(summary) => summary,
        Err(error) => return Ok(WaveformState::Failed(error)),
    };
    drop(lease);
    let bars = reduce(item, span, columns, &summary)?;
    Ok(WaveformState::Ready(WaveformDisplay { bars }))
}

fn reduce(
    item: &project::Item,
    span: TimeRange,
    columns: usize,
    summary: &PeakSummary,
) -> Result<Vec<WaveformBar>, EditError> {
    let duration = i128::from(span.duration().0);
    let start = i128::from(span.start().0);
    let columns = i128::try_from(columns)
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    let mut bars = Vec::with_capacity(
        usize::try_from(columns)
            .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?,
    );
    for index in 0..columns {
        let output_start = start
            .checked_add(
                duration.checked_mul(index).ok_or(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ))? / columns,
            )
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
        let output_end = start
            .checked_add(
                duration
                    .checked_mul(index.checked_add(1).ok_or(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    ))?)
                    .ok_or(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    ))?
                    / columns,
            )
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
        let source_start = item.source_time_at(tick(output_start)?)?;
        let source_end = item
            .source_time_at(tick(output_end)?)?
            .min(item.source_range.end())
            .max(item.source_range.start());
        let source_start = source_start.max(item.source_range.start());
        let buckets = (source_start < source_end)
            .then(|| SourceRange::new(source_start, source_end).ok())
            .flatten()
            .and_then(|range| summary.bucket_span(range));
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        if let Some(buckets) = buckets {
            for bucket in buckets {
                for channel in 0..summary.channels() {
                    if let Some(peak) = summary.peak(bucket, channel) {
                        min = min.min(peak.min);
                        max = max.max(peak.max);
                    }
                }
            }
        }
        bars.push(if min.is_finite() && max.is_finite() {
            WaveformBar {
                min,
                max,
                over: min < -1.0 || max > 1.0,
            }
        } else {
            WaveformBar {
                min: 0.0,
                max: 0.0,
                over: false,
            }
        });
    }
    Ok(bars)
}

fn tick(value: i128) -> Result<Tick, EditError> {
    Ok(Tick(i64::try_from(value).map_err(|_| {
        EditError::Control(crate::session::wire::ControlError::Limit)
    })?))
}

fn audio_info(asset: &SourceAsset) -> Result<&AudioInfo, EditError> {
    asset
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Audio)
        .and_then(|stream| stream.audio.as_ref())
        .ok_or(EditError::Unsupported("waveform requires audio"))
}

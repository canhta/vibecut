//! Independent source-edge snapping for ordered, potentially reordered shorts.
use crate::error::{BoundaryFailure, Error, Result as ShortsResult};
use ai::clips::{ClipRole, ClipSourceSegment};
use captions::{LanguageProfile, Transcript};
use engine::{EditError, Result};
use media::SpeechEnergyWindow;
use serde::Serialize;
use time::{FrameRate, SourceRange, Tick};

/// All durations and deltas are source ticks. Bounds are inclusive; the fallback
/// quantization allowance is documented on `tolerance`.
#[derive(Clone, Debug)]
pub struct ClipBoundarySettings {
    pub source_range: SourceRange,
    pub frame_rate: FrameRate,
    /// Word-gap search radius; sub-frame fallback adds at most half a frame.
    pub tolerance: Tick,
    /// Mono RMS amplitude ratio in 0..=1; energy is a hint, never overrides words.
    pub silence_rms: f64,
    pub min_duration: Tick,
    pub max_duration: Tick,
    pub min_segment_duration: Tick,
    /// User-selected limit in 1..=32 (the suggestion default is four).
    pub max_segments: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum BoundaryKind {
    SentenceSilence,
    Sentence,
    Silence,
    WordOrSegment,
    /// Frame-rounded word gap; export must apply a short audio fade.
    WordFade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BoundaryAdjustment {
    pub original: Tick,
    pub adjusted: Tick,
    pub delta: Tick,
    pub kind: BoundaryKind,
}

#[derive(Clone, Debug, Serialize)]
pub struct SegmentAdjustment {
    pub start: BoundaryAdjustment,
    pub end: BoundaryAdjustment,
}

#[derive(Clone, Debug, Serialize)]
pub struct SnappedShort {
    /// Playback order and editorial roles are preserved.
    pub segments: Vec<ClipSourceSegment>,
    /// One pair of signed deltas per segment, in playback order.
    pub adjustments: Vec<SegmentAdjustment>,
    pub duration: Tick,
}

struct Gap {
    start: Tick,
    end: Tick,
    sentence: bool,
}

/// Adjust explicit shorts without suggestion generation, IO or project mutation.
/// Captions owns safe word/span grouping and profile punctuation; media owns RMS.
/// Prefer sentence + silence, sentence, silence, then safe word/segment gaps,
/// ranking word gaps by measured RMS, distance and earlier time. If no safe frame
/// exists, round the measured gap to its nearest frame (at most half-frame
/// speech overlap), reporting WordFade for the clip exporter’s 10 ms audio ramp. Incomplete timing protects the whole segment.
/// A short that becomes overlapping, too short/long or has no safe frame is
/// refused atomically with its playback index; input is never changed.
/// At most 32 shorts, 32 segments/short and 360,001 energy windows are admitted.
pub fn snap_clip_boundaries(
    shorts: &[Vec<ClipSourceSegment>],
    transcript: &Transcript,
    profile: &LanguageProfile,
    energy: &[SpeechEnergyWindow],
    settings: &ClipBoundarySettings,
) -> ShortsResult<Vec<SnappedShort>> {
    validate_settings(settings)?;
    if shorts.len() > 32 || energy.len() > 360_001 {
        return Err(EditError::Control(engine::control::ControlError::Limit).into());
    }
    let units = captions::source_speech_units(transcript, profile).map_err(Error::kernel)?;
    if units.is_empty() {
        return Err(EditError::Invalid("empty boundary transcript").into());
    }
    let mut gaps = Vec::new();
    let mut previous = settings.source_range.start();
    let mut sentence = true;
    for unit in &units {
        if unit.range.start() < previous || unit.range.end() > settings.source_range.end() {
            return Err(EditError::Invalid("boundary transcript outside source").into());
        }
        gaps.push(Gap {
            start: previous,
            end: unit.range.start(),
            sentence,
        });
        previous = unit.range.end();
        sentence = unit.sentence_end;
    }
    gaps.push(Gap {
        start: previous,
        end: settings.source_range.end(),
        sentence: true,
    });
    let silence = silence_ranges(energy, settings)?;
    let mut output = Vec::with_capacity(shorts.len());
    for (short, segments) in shorts.iter().enumerate() {
        validate_candidate(segments, settings).map_err(|reason| failure(short, 0, reason))?;
        let mut result = SnappedShort {
            segments: Vec::new(),
            adjustments: Vec::new(),
            duration: Tick::ZERO,
        };
        for (index, segment) in segments.iter().enumerate() {
            let start = snap_edge(segment.range.start(), &gaps, &silence, energy, settings)?
                .ok_or_else(|| failure(short, index, BoundaryFailure::NoSafeFrame))?;
            let end = snap_edge(segment.range.end(), &gaps, &silence, energy, settings)?
                .ok_or_else(|| failure(short, index, BoundaryFailure::NoSafeFrame))?;
            if end.adjusted <= start.adjusted
                || end.adjusted - start.adjusted < settings.min_segment_duration
            {
                return Err(failure(short, index, BoundaryFailure::SegmentDuration));
            }
            let mut adjusted = segment.clone();
            adjusted.range =
                SourceRange::new(start.adjusted, end.adjusted).map_err(Error::kernel)?;
            result.duration = result
                .duration
                .checked_add(adjusted.range.duration())
                .ok_or(EditError::Control(engine::control::ControlError::Limit))?;
            result.segments.push(adjusted);
            result.adjustments.push(SegmentAdjustment { start, end });
        }
        validate_ranges(&result.segments).map_err(|reason| failure(short, 0, reason))?;
        if result.duration < settings.min_duration || result.duration > settings.max_duration {
            return Err(failure(short, 0, BoundaryFailure::TotalDuration));
        }
        output.push(result);
    }
    Ok(output)
}

fn failure(short: usize, segment: usize, reason: BoundaryFailure) -> Error {
    Error::ClipBoundary {
        short,
        segment,
        reason,
    }
}

fn validate_settings(s: &ClipBoundarySettings) -> Result<()> {
    if s.source_range.start() < Tick::ZERO
        || s.tolerance < Tick::ZERO
        || !s.silence_rms.is_finite()
        || !(0.0..=1.0).contains(&s.silence_rms)
        || s.min_duration <= Tick::ZERO
        || s.max_duration < s.min_duration
        || s.min_segment_duration <= Tick::ZERO
        || s.min_segment_duration > s.max_duration
        || !(1..=32).contains(&s.max_segments)
    {
        return Err(EditError::Invalid("clip boundary settings"));
    }
    Ok(())
}

fn validate_candidate(
    segments: &[ClipSourceSegment],
    s: &ClipBoundarySettings,
) -> std::result::Result<(), BoundaryFailure> {
    if segments.is_empty() || segments.len() > s.max_segments {
        return Err(BoundaryFailure::Candidate);
    }
    let hooks: Vec<_> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.role == ClipRole::Hook)
        .collect();
    if !hooks.is_empty() && (hooks.len() != 1 || hooks.first().is_none_or(|(i, _)| *i != 0)) {
        return Err(BoundaryFailure::Candidate);
    }
    if segments
        .iter()
        .any(|segment| segment.range.intersect(s.source_range) != Some(segment.range))
    {
        return Err(BoundaryFailure::Candidate);
    }
    validate_ranges(segments)
}

fn validate_ranges(segments: &[ClipSourceSegment]) -> std::result::Result<(), BoundaryFailure> {
    let mut source: Vec<_> = segments.iter().collect();
    source.sort_by_key(|s| s.range.start());
    for (rank, segment) in source.iter().enumerate() {
        if segment.source_order != rank {
            return Err(BoundaryFailure::Candidate);
        }
        if let Some(next) = source.get(rank + 1)
            && segment.range.overlaps(next.range)
        {
            return Err(BoundaryFailure::SegmentOverlap);
        }
    }
    Ok(())
}

fn silence_ranges(
    windows: &[SpeechEnergyWindow],
    s: &ClipBoundarySettings,
) -> Result<Vec<SourceRange>> {
    let mut result: Vec<SourceRange> = Vec::new();
    let mut previous = Tick::ZERO;
    for window in windows {
        if !window.rms.is_finite() || window.rms < 0.0 || window.range.start() < previous {
            return Err(EditError::Invalid("speech energy windows"));
        }
        previous = window.range.end();
        if window.rms > s.silence_rms {
            continue;
        }
        let Some(range) = window.range.intersect(s.source_range) else {
            continue;
        };
        if let Some(last) = result.last_mut()
            && last.end() == range.start()
        {
            *last = SourceRange::new(last.start(), range.end())?;
        } else {
            result.push(range);
        }
    }
    Ok(result)
}

fn snap_edge(
    original: Tick,
    gaps: &[Gap],
    silence: &[SourceRange],
    energy: &[SpeechEnergyWindow],
    s: &ClipBoundarySettings,
) -> Result<Option<BoundaryAdjustment>> {
    let low = original
        .saturating_sub(s.tolerance)
        .max(s.source_range.start());
    let high = original
        .saturating_add(s.tolerance)
        .min(s.source_range.end());
    let mut best: Option<(u8, u64, i64, Tick)> = None;
    let mut consider = |lo: Tick, hi: Tick, tier: u8, rms: f64| -> Result<()> {
        let Some(tick) = safe_frame(lo.max(low), hi.min(high), original, s.frame_rate)? else {
            return Ok(());
        };
        let key = (
            tier,
            if rms == 0.0 { 0 } else { rms.to_bits() },
            (tick - original).abs().0,
            tick,
        );
        if best.is_none_or(|old| key < old) {
            best = Some(key);
        }
        Ok(())
    };
    for gap in gaps
        .iter()
        .skip(gaps.partition_point(|g| g.end < low))
        .take_while(|g| g.start <= high)
    {
        consider(
            gap.start,
            gap.end,
            if gap.sentence { 1 } else { 3 },
            if gap.sentence { 0.0 } else { f64::MAX },
        )?;
        let lo = gap.start.max(low);
        let hi = gap.end.min(high);
        for quiet in silence
            .iter()
            .skip(silence.partition_point(|r| r.end() < lo))
            .take_while(|r| r.start() <= hi)
        {
            if !gap.sentence {
                continue;
            }
            consider(
                gap.start.max(quiet.start()),
                gap.end.min(quiet.end()),
                0,
                0.0,
            )?;
        }
    }
    // Rank measured inter-word energy even when no window is below the
    // absolute silence threshold (normal continuous speech).
    for gap in gaps
        .iter()
        .skip(gaps.partition_point(|g| g.end < low))
        .take_while(|g| g.start <= high)
        .filter(|g| !g.sentence)
    {
        for window in energy
            .iter()
            .skip(energy.partition_point(|w| w.range.end() < gap.start.max(low)))
            .take_while(|w| w.range.start() <= gap.end.min(high))
        {
            consider(
                gap.start.max(window.range.start()),
                gap.end.min(window.range.end()),
                if window.rms <= s.silence_rms { 2 } else { 3 },
                window.rms,
            )?;
        }
    }
    // A sub-frame word gap still identifies a measured speech boundary. Keep
    // video on its frame grid, with at most half a frame of speech overlap.
    if best.is_none() {
        let mut fallback: Option<(u64, i64, Tick)> = None;
        for gap in gaps
            .iter()
            .skip(gaps.partition_point(|g| g.end < low))
            .take_while(|g| g.start <= high)
        {
            let lo = gap.start.max(low);
            let hi = gap.end.min(high);
            if hi < lo {
                continue;
            }
            let mut consider = |target: Tick, rms: f64| -> Result<()> {
                let tick = s.frame_rate.snap_nearest(target)?;
                if tick < s.source_range.start() || tick > s.source_range.end() {
                    return Ok(());
                }
                let key = (
                    if rms == 0.0 { 0 } else { rms.to_bits() },
                    (tick - original).abs().0,
                    tick,
                );
                if fallback.is_none_or(|old| key < old) {
                    fallback = Some(key);
                }
                Ok(())
            };
            let mut measured = false;
            for window in energy
                .iter()
                .skip(energy.partition_point(|w| w.range.end() < lo))
                .take_while(|w| w.range.start() <= hi)
            {
                measured = true;
                consider(
                    original.clamp(lo.max(window.range.start()), hi.min(window.range.end())),
                    window.rms,
                )?;
            }
            if !measured {
                consider(original.clamp(lo, hi), 1.0)?;
            }
        }
        if let Some((_, distance, tick)) = fallback {
            best = Some((4, 0, distance, tick));
        }
    }
    Ok(best.map(|(tier, _, _, adjusted)| BoundaryAdjustment {
        original,
        adjusted,
        delta: adjusted - original,
        kind: match tier {
            0 => BoundaryKind::SentenceSilence,
            1 => BoundaryKind::Sentence,
            2 => BoundaryKind::Silence,
            3 => BoundaryKind::WordOrSegment,
            _ => BoundaryKind::WordFade,
        },
    }))
}

fn safe_frame(low: Tick, high: Tick, target: Tick, fps: FrameRate) -> Result<Option<Tick>> {
    if high < low {
        return Ok(None);
    }
    let floor = fps.snap(low)?;
    let first = if floor < low {
        fps.tick_of(
            fps.frame_at(low)?
                .checked_add(1)
                .ok_or(EditError::Control(engine::control::ControlError::Limit))?,
        )?
    } else {
        floor
    };
    let last = fps.snap(high)?;
    if first > last {
        return Ok(None);
    }
    Ok(Some(
        fps.snap_nearest(target.clamp(first, last))?
            .clamp(first, last),
    ))
}

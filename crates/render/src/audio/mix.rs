//! Mixes project audio into bounded output blocks.

use super::cleanup::CleanupChain;
use super::retime::{Pitch, Retimer};
use crate::{
    Error, ExecutionHooks, MemoryKind, Reservation, Result, SourceProvider, check, vector,
};
use project::{ItemId, ItemParams, Project};
use std::collections::BTreeMap;
use time::{Rounding, Tick};
/// Planar f32 audio on the project output clock.
/// Stereo default: mono duplicates at unity; stereo->mono averages L/R.
pub struct AudioBlock {
    pub(crate) rate: u32,
    pub(crate) planes: Vec<Vec<f32>>,
    _lease: Box<dyn Reservation>,
}
impl AudioBlock {
    pub fn new(
        rate: u32,
        channels: u16,
        frames: usize,
        kind: MemoryKind,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        if !(8000..=192000).contains(&rate) || !matches!(channels, 1 | 2) || frames > 65536 {
            return Err(Error::Invalid("audio block format"));
        }
        let lease = hooks.reserve(kind, frames as u64 * u64::from(channels) * 4)?;
        let planes = (0..channels)
            .map(|_| vector(frames, 0.0))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            rate,
            planes,
            _lease: lease,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn planes(&self) -> &[Vec<f32>] {
        &self.planes
    }
    pub fn planes_mut(&mut self) -> &mut [Vec<f32>] {
        &mut self.planes
    }
    pub fn peak(&self) -> f32 {
        self.planes
            .iter()
            .flatten()
            .fold(0.0, |peak, s| peak.max(s.abs()))
    }
    pub fn samples(&self) -> usize {
        self.planes.first().map_or(0, Vec::len)
    }
}
/// What `mix_raw` applies besides item volume: loudness gains, the master fader
/// and an optional solo item. Production reads the project; measurement passes
/// build their own so pass one never sees gains or the fader.
pub(crate) struct Chain<'a> {
    clip: &'a [(ItemId, f32)],
    master_db: f32,
    fader: bool,
    solo: Option<ItemId>,
}
impl<'a> Chain<'a> {
    /// The gains and fader the project requests, for preview and export alike.
    pub(crate) fn of(project: &'a Project) -> Self {
        let l = &project.data().settings.loudness;
        let (clip, master_db) = match &l.measured {
            Some(m) => (
                if l.clip.is_some() { &m.clip[..] } else { &[] },
                if l.master.is_some() { m.master_db } else { 0.0 },
            ),
            None => (&[][..], 0.0),
        };
        Self {
            clip,
            master_db,
            fader: true,
            solo: None,
        }
    }
    /// No fader or mute; `clip` gains applied, only `solo` audible when set.
    pub(crate) fn measuring(clip: &'a [(ItemId, f32)], solo: Option<ItemId>) -> Self {
        Self {
            clip,
            master_db: 0.0,
            fader: false,
            solo,
        }
    }
    fn clip_db(&self, item: ItemId) -> f32 {
        self.clip
            .binary_search_by_key(&item, |(id, _)| *id)
            .map_or(0.0, |i| self.clip.get(i).map_or(0.0, |(_, g)| *g))
    }
}

/// Absolute output sample window [start,start+frames), using nearest rounding for
/// every item/source boundary once. Identical sample math for preview and export.
pub fn mix_audio(
    project: &Project,
    start: i64,
    frames: usize,
    channels: u16,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
) -> Result<AudioBlock> {
    AudioMixer::new(project, channels, 0, hooks)?.mix(start, frames, sources, hooks)
}

/// Stateful shared graph for realtime blocks and offline export. A discontinuous
/// seek replays from timeline zero, so its envelope is independent of request history.
/// Keep one mixer per immutable snapshot; construction reserves bounded DSP state.
pub struct AudioMixer<'a> {
    project: &'a Project,
    channels: u16,
    fade_ms: u16,
    limiter: super::limiter::Limiter,
    retimers: BTreeMap<project::ItemId, Retimer>,
    cleanups: BTreeMap<project::ItemId, CleanupChain>,
    next: Option<i64>,
    _lease: Box<dyn Reservation>,
}
impl<'a> AudioMixer<'a> {
    pub fn new(
        project: &'a Project,
        channels: u16,
        fade_ms: u16,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        if !matches!(channels, 1 | 2) || fade_ms > 1000 {
            return Err(Error::Invalid("mixer settings"));
        }
        let rate = project.data().settings.audio_sample_rate;
        // Rings and monotonic deque, including capacity overhead, at <=192 kHz.
        let lease = hooks.reserve(MemoryKind::Compositor, 128 * 1024)?;
        Ok(Self {
            project,
            channels,
            fade_ms,
            limiter: super::limiter::Limiter::new(rate, channels),
            retimers: BTreeMap::new(),
            cleanups: BTreeMap::new(),
            next: None,
            _lease: lease,
        })
    }
    /// Processing delay, fully removed from returned PCM timestamps and samples.
    pub fn latency_samples(&self) -> usize {
        self.limiter.latency()
    }

    pub fn mix(
        &mut self,
        start: i64,
        frames: usize,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        check(hooks)?;
        if frames > 65536 {
            return Err(Error::Invalid("audio block format"));
        }
        let end = start
            .checked_add(frames as i64)
            .ok_or(Error::Invalid("sample window"))?;
        let latency = self.latency_samples() as i64;
        end.checked_add(latency)
            .ok_or(Error::Invalid("limiter sample window"))?;
        if frames == 0
            || start
                >= self.project.duration().to_units(
                    i64::from(self.project.data().settings.audio_sample_rate),
                    Rounding::Nearest,
                )?
        {
            self.next = None;
            return AudioBlock::new(
                self.project.data().settings.audio_sample_rate,
                self.channels,
                frames,
                MemoryKind::Compositor,
                hooks,
            );
        }
        let result = (|| {
            if self.next != Some(start) {
                self.limiter = super::limiter::Limiter::new(
                    self.project.data().settings.audio_sample_rate,
                    self.channels,
                );
                // Retime state replays with the timeline from zero like the limiter.
                self.retimers.clear();
                self.cleanups.clear();
                // Prime delay from zero (or earlier silence for negative windows).
                let origin = start.min(0);
                self.next = Some(origin);
                let mut prime = mix_raw(
                    self.project,
                    origin,
                    latency as usize,
                    self.channels,
                    sources,
                    hooks,
                    self.fade_ms,
                    &Chain::of(self.project),
                    &mut self.retimers,
                    &mut self.cleanups,
                )?;
                self.limiter.process(&mut prime.planes);
                while self.next.is_some_and(|next| next < start) {
                    let next = self.next.ok_or(Error::Invalid("mixer cursor"))?;
                    let count = (start - next).min(65536) as usize;
                    self.run(next, count, sources, hooks)?;
                    self.next = Some(next + count as i64);
                }
            }
            let out = self.run(start, frames, sources, hooks)?;
            self.next = Some(end);
            Ok(out)
        })();
        if result.is_err() {
            self.next = None;
        }
        result
    }
    fn run(
        &mut self,
        start: i64,
        frames: usize,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = mix_raw(
            self.project,
            start + self.latency_samples() as i64,
            frames,
            self.channels,
            sources,
            hooks,
            self.fade_ms,
            &Chain::of(self.project),
            &mut self.retimers,
            &mut self.cleanups,
        )?;
        self.limiter.process(&mut out.planes);
        Ok(out)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn mix_raw(
    project: &Project,
    start: i64,
    frames: usize,
    channels: u16,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    fade_ms: u16,
    chain: &Chain<'_>,
    retimers: &mut BTreeMap<project::ItemId, Retimer>,
    cleanups: &mut BTreeMap<project::ItemId, CleanupChain>,
) -> Result<AudioBlock> {
    check(hooks)?;
    let rate = project.data().settings.audio_sample_rate;
    let end = start
        .checked_add(i64::try_from(frames).map_err(|_| Error::Invalid("sample count"))?)
        .ok_or(Error::Invalid("sample window"))?;
    let mut out = AudioBlock::new(rate, channels, frames, MemoryKind::Compositor, hooks)?;
    if chain.fader && project.data().settings.master_muted {
        return Ok(out);
    }
    let mut spans = Vec::new();
    for track in project.data().scene().timeline.iter() {
        if track.muted {
            continue;
        }
        for item in &track.items {
            if !has_source_audio(item) || chain.solo.is_some_and(|id| id != item.id) {
                continue;
            }
            let i0 = item.start.to_units(i64::from(rate), Rounding::Nearest)?;
            let i1 = item.end()?.to_units(i64::from(rate), Rounding::Nearest)?;
            let a = start.max(i0);
            let b = end.min(i1);
            if b <= a {
                continue;
            }
            let Some(id) = item.source_asset else {
                return Err(Error::Invalid("audio source identity"));
            };
            let asset = project
                .data()
                .assets
                .iter()
                .find(|a| a.id() == id)
                .ok_or(Error::Invalid("audio asset"))?;
            if !asset
                .probe()
                .streams
                .iter()
                .any(|s| s.kind == media::StreamKind::Audio)
            {
                continue;
            }
            if spans.len() >= 4096 {
                return Err(Error::Invalid("audio window expansion"));
            }
            spans.push((item, asset, i0, i1, a, b));
        }
    }
    let active: Vec<_> = spans.iter().map(|(item, _, _, _, _, _)| item.id).collect();
    sources.begin_audio(&active);
    retimers.retain(|id, _| active.contains(id));
    cleanups.retain(|id, _| active.contains(id));
    for (item, asset, i0, i1, a, b) in spans {
        check(hooks)?;
        let window = item.retime_window()?;
        let output_start = item
            .start
            .checked_sub(window.offset)
            .ok_or(Error::Invalid("audio continuation origin"))?;
        let output_origin = output_start.to_units(i64::from(rate), Rounding::Nearest)?;
        let output_end = output_start
            .checked_add(
                item.retime
                    .speed
                    .output_duration(window.source_range.duration())?,
            )
            .ok_or(Error::Invalid("audio continuation end"))?
            .to_units(i64::from(rate), Rounding::Nearest)?;
        let origin = window
            .source_range
            .start()
            .to_units(i64::from(rate), Rounding::Nearest)?;
        let mut input = if item.retime.speed.is_unity() && !item.retime.reverse {
            let source = origin
                .checked_add(a - output_origin)
                .ok_or(Error::Invalid("source sample position"))?;
            sources.audio(asset, item.id, source, (b - a) as usize, rate, hooks)?
        } else {
            let length = window
                .source_range
                .end()
                .to_units(i64::from(rate), Rounding::Nearest)?
                - origin;
            // A reversed item retimes the window's samples reversed, so the reversal
            // lands before rate conversion and time stretch: pitch is kept as usual.
            // At 1x the identity resampler (Follow, ratio 1) is exact.
            let pitch = if item.retime.maintain_pitch && !item.retime.speed.is_unity() {
                Pitch::Keep
            } else {
                Pitch::Follow
            };
            let speed = item.retime.speed.ratio();
            if retimers
                .get(&item.id)
                .is_none_or(|r| !r.matches(rate, speed, pitch, length))
            {
                retimers.insert(item.id, Retimer::new(rate, speed, pitch, length, hooks)?);
            }
            let retimer = retimers
                .get_mut(&item.id)
                .ok_or(Error::Invalid("retime state"))?;
            let reverse = item.retime.reverse;
            let mut pull = |at: i64, count: usize| -> Result<Vec<Vec<f32>>> {
                if !reverse {
                    return Ok(sources
                        .audio(asset, item.id, origin + at, count, rate, hooks)?
                        .planes);
                }
                // Reversed sample `n` is window sample `length - 1 - n`: the block
                // `[at, at + count)` is window `[length - at - count, length - at)`, reversed.
                let first = length
                    .checked_sub(at)
                    .and_then(|t| t.checked_sub(count as i64))
                    .ok_or(Error::Invalid("reversed source position"))?;
                let mut planes = sources
                    .audio(asset, item.id, origin + first, count, rate, hooks)?
                    .planes;
                for plane in &mut planes {
                    plane.reverse();
                }
                Ok(planes)
            };
            let planes = retimer.read(a - output_origin, (b - a) as usize, &mut pull, hooks)?;
            let mut block = AudioBlock::new(
                rate,
                u16::try_from(planes.len()).map_err(|_| Error::Invalid("source audio block"))?,
                (b - a) as usize,
                MemoryKind::Decoded,
                hooks,
            )?;
            for (dst, src) in block.planes.iter_mut().zip(&planes) {
                dst.copy_from_slice(src);
            }
            block
        };
        if input.rate != rate
            || input.samples() != (b - a) as usize
            || !matches!(input.planes.len(), 1 | 2)
            || input
                .planes
                .iter()
                .any(|p| p.iter().any(|s| !s.is_finite()))
        {
            return Err(Error::Invalid("source audio block"));
        }
        if let Some(cleanup) = cleanup_of(item) {
            cleanups
                .entry(item.id)
                .or_insert_with(|| CleanupChain::new(cleanup, rate))
                .process_at(a, &mut input.planes);
        }
        let volume_path = project::AnimationPath::from(project::BuiltinPath::Volume);
        let muted_path = project::AnimationPath::from(project::BuiltinPath::Muted);
        let (volume, muted, fades) = match &item.content {
            ItemParams::Audio(p) => (p.volume, p.muted, p.fades),
            ItemParams::Video(p) => (p.volume, p.muted, p.fades),
            _ => (0.0, true, project::Fades::default()),
        };
        // Clip fades multiply the volume curve before summing and the master limiter.
        let fade_spans = fades.spans(item.start, item.end()?, i64::from(rate))?;
        // Evaluate on absolute output samples, never relative to the block or
        // another track's active span. Gain is identical across all adapters.
        let _gains = hooks.reserve(MemoryKind::Compositor, (b - a) as u64 * 4)?;
        let mut gains = vector((b - a) as usize, 0.0)?;
        for (n, gain) in gains.iter_mut().enumerate() {
            let local = Tick::from_units(a + n as i64, i64::from(rate), Rounding::Nearest)?
                .checked_sub(item.start)
                .ok_or(Error::Invalid("automation tick"))?;
            let muted = match item.animations.value_at(&muted_path, local)? {
                Some(project::ParamValue::Bool(value)) => value,
                _ => muted,
            };
            let db = match item.animations.value_at(&volume_path, local)? {
                Some(project::ParamValue::Scalar(value)) => value.clamp(-60.0, 20.0) as f32,
                _ => volume,
            };
            if !muted {
                let master = if chain.fader {
                    project.data().settings.master_volume
                } else {
                    0.0
                };
                *gain = 10f32.powf((db + master + chain.clip_db(item.id)) / 20.0);
            }
        }
        let offset = (a - start) as usize;
        let fade =
            (i64::from(rate) * i64::from(fade_ms) / 1000).min((output_end - output_origin) / 2);
        for (channel, destination) in out.planes.iter_mut().enumerate() {
            for (n, dst) in destination
                .iter_mut()
                .skip(offset)
                .take((b - a) as usize)
                .enumerate()
            {
                let get = |c: usize| {
                    input
                        .planes
                        .get(c)
                        .and_then(|p| p.get(n))
                        .copied()
                        .unwrap_or(0.0)
                };
                let value = if input.planes.len() == 1 {
                    get(0)
                } else if channels == 1 {
                    (get(0) + get(1)) * 0.5
                } else {
                    get(channel)
                };
                let sample = a - i0 + n as i64;
                let envelope = if fade > 0 {
                    let original_sample = a - output_origin + n as i64;
                    (original_sample.min(output_end - output_origin - 1 - original_sample) as f32
                        / fade as f32)
                        .clamp(0.0, 1.0)
                } else {
                    1.0
                };
                *dst += value
                    * gains.get(n).copied().ok_or(Error::Invalid("gain sample"))?
                    * envelope
                    * fade_spans.gain(sample);
                if !dst.is_finite() {
                    return Err(Error::Invalid("audio mix overflow"));
                }
            }
        }
        if b == i1 {
            sources.finish_audio(item.id);
            retimers.remove(&item.id);
            cleanups.remove(&item.id);
        }
    }
    if chain.master_db != 0.0 {
        let gain = 10f32.powf(chain.master_db / 20.0);
        for sample in out.planes.iter_mut().flatten() {
            *sample *= gain;
            if !sample.is_finite() {
                return Err(Error::Invalid("audio mix overflow"));
            }
        }
    }
    Ok(out)
}

/// The cleanup chain of an item's source audio; other item kinds have none.
fn cleanup_of(item: &project::Item) -> Option<&project::AudioCleanup> {
    match &item.content {
        ItemParams::Audio(p) => Some(&p.cleanup),
        ItemParams::Video(p) => Some(&p.cleanup),
        _ => None,
    }
}

/// Conservative source admission: a held mute with no unmute key needs no IO.
/// Any possible unmute keeps the source admitted, including before/after the window.
pub(crate) fn has_source_audio(item: &project::Item) -> bool {
    let muted = match &item.content {
        ItemParams::Video(p) if p.source_audio_enabled => p.muted,
        ItemParams::Audio(p) => p.muted,
        _ => return false,
    };
    if let Some((_, project::Param::Value(channel))) = item
        .animations
        .iter()
        .find(|(path, _)| path.builtin() == Some(project::BuiltinPath::Muted))
    {
        channel.base() == &project::ParamValue::Bool(false)
            || channel
                .keys()
                .iter()
                .any(|key| key.value == project::ParamValue::Bool(false))
    } else {
        !muted
    }
}

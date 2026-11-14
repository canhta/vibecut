//! Mixes project audio into bounded output blocks.

use super::cleanup::CleanupChain;
use super::retime::{Pitch, RateMap, Retimer};
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
    /// A compound clip's nested mix: no gains or fader, everything audible. The compound
    /// clip's own volume, fades and loudness gain apply once its mix is its source.
    fn nested() -> Chain<'static> {
        Chain {
            clip: &[],
            master_db: 0.0,
            fader: false,
            solo: None,
        }
    }
    fn clip_db(&self, item: ItemId) -> f32 {
        self.clip
            .binary_search_by_key(&item, |(id, _)| *id)
            .map_or(0.0, |i| self.clip.get(i).map_or(0.0, |(_, g)| *g))
    }
}

/// Streaming state of one mix level: retimers and cleanup chains by item, and the state of
/// each compound clip's nested mix, so every level streams across consecutive blocks.
#[derive(Default)]
pub(crate) struct MixState {
    retimers: BTreeMap<ItemId, Retimer>,
    cleanups: BTreeMap<ItemId, CleanupChain>,
    nested: BTreeMap<ItemId, MixState>,
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
    state: MixState,
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
            state: MixState::default(),
            next: None,
            _lease: lease,
        })
    }
    /// Processing delay, fully removed from returned PCM timestamps and samples.
    pub fn latency_samples(&self) -> usize {
        self.limiter.latency()
    }

    /// Delay of the longest cleanup chain of any item, in samples (spectral noise reduction and
    /// de-click delay their signal). The graph already compensates it by reading that far ahead
    /// in each such item's source, so it never shifts returned PCM and `latency_samples` does not
    /// include it. Zero when no item has a delaying effect.
    pub fn cleanup_latency_samples(&self) -> usize {
        let rate = self.project.data().settings.audio_sample_rate;
        self.project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .filter_map(cleanup_of)
            .map(|cleanup| super::cleanup::latency_samples(cleanup, rate))
            .max()
            .unwrap_or(0)
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
                self.state = MixState::default();
                // Prime delay from zero (or earlier silence for negative windows).
                let origin = start.min(0);
                self.next = Some(origin);
                let mut prime = mix_raw(
                    self.level(hooks),
                    &self.project.data().scene().timeline,
                    origin,
                    latency as usize,
                    sources,
                    &Chain::of(self.project),
                    &mut self.state,
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
    /// The scene level of this mixer's output.
    fn level<'h>(&self, hooks: &'h dyn ExecutionHooks) -> Level<'h>
    where
        'a: 'h,
    {
        Level {
            project: self.project,
            channels: self.channels,
            fade_ms: self.fade_ms,
            depth: 0,
            hooks,
        }
    }
    fn run(
        &mut self,
        start: i64,
        frames: usize,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = mix_raw(
            self.level(hooks),
            &self.project.data().scene().timeline,
            start + self.latency_samples() as i64,
            frames,
            sources,
            &Chain::of(self.project),
            &mut self.state,
        )?;
        self.limiter.process(&mut out.planes);
        Ok(out)
    }
}

/// One level of the mix: `timeline` is the scene at depth 0, or a compound clip's nested
/// timeline, whose mix is that clip's source audio.
pub(crate) fn mix_raw(
    level: Level<'_>,
    timeline: &project::Timeline,
    start: i64,
    frames: usize,
    sources: &mut dyn SourceProvider,
    chain: &Chain<'_>,
    state: &mut MixState,
) -> Result<AudioBlock> {
    let Level {
        project,
        channels,
        fade_ms,
        depth,
        hooks,
    } = level;
    check(hooks)?;
    if depth > project::MAX_COMPOUND_DEPTH {
        return Err(Error::Invalid("compound nesting"));
    }
    let rate = project.data().settings.audio_sample_rate;
    let end = start
        .checked_add(i64::try_from(frames).map_err(|_| Error::Invalid("sample count"))?)
        .ok_or(Error::Invalid("sample window"))?;
    let mut out = AudioBlock::new(rate, channels, frames, MemoryKind::Compositor, hooks)?;
    if chain.fader && project.data().settings.master_muted {
        return Ok(out);
    }
    let mut spans = Vec::new();
    for track in timeline.iter() {
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
            // A compound clip's source is its nested mix; every other item reads its asset.
            let asset = if matches!(item.content, ItemParams::Compound(_)) {
                None
            } else {
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
                Some(asset)
            };
            if spans.len() >= 4096 {
                return Err(Error::Invalid("audio window expansion"));
            }
            spans.push((item, asset, i0, i1, a, b));
        }
    }
    let active: Vec<_> = spans.iter().map(|(item, _, _, _, _, _)| item.id).collect();
    // The scene level names the source cursors it keeps, compound members included; a nested
    // level reads within that set.
    if depth == 0 {
        let mut kept = active.clone();
        for (item, _, _, _, _, _) in &spans {
            if let ItemParams::Compound(params) = &item.content {
                kept.extend(
                    params
                        .timeline
                        .all_tracks()
                        .into_iter()
                        .flat_map(|track| &track.items)
                        .map(|member| member.id),
                );
            }
        }
        sources.begin_audio(&kept);
    }
    state.retimers.retain(|id, _| active.contains(id));
    state.cleanups.retain(|id, _| active.contains(id));
    state.nested.retain(|id, _| active.contains(id));
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
                    .output_duration(window.source_range.duration())?,
            )
            .ok_or(Error::Invalid("audio continuation end"))?
            .to_units(i64::from(rate), Rounding::Nearest)?;
        let origin = window
            .source_range
            .start()
            .to_units(i64::from(rate), Rounding::Nearest)?;
        // The item's source samples `[lo, hi)` on the output clock, retimed and validated.
        let mut read = |lo: i64, hi: i64| -> Result<AudioBlock> {
            let count = (hi - lo) as usize;
            let input = if item.retime.speed.is_unity()
                && item.retime.curve.is_none()
                && !item.retime.reverse
            {
                let source = origin
                    .checked_add(lo - output_origin)
                    .ok_or(Error::Invalid("source sample position"))?;
                source_audio(
                    level,
                    item,
                    asset,
                    source,
                    count,
                    sources,
                    &mut state.nested,
                )?
            } else {
                let length = window
                    .source_range
                    .end()
                    .to_units(i64::from(rate), Rounding::Nearest)?
                    - origin;
                // A reversed item retimes the window's samples reversed, so the reversal
                // lands before rate conversion and time stretch: pitch is kept as usual.
                // At 1x the identity resampler (Follow, ratio 1) is exact.
                let pitch = if item.retime.maintain_pitch
                    && (item.retime.curve.is_some() || !item.retime.speed.is_unity())
                {
                    Pitch::Keep
                } else {
                    Pitch::Follow
                };
                let map = rate_map(item, rate)?;
                if state
                    .retimers
                    .get(&item.id)
                    .is_none_or(|r| !r.matches(rate, &map, pitch, length))
                {
                    state
                        .retimers
                        .insert(item.id, Retimer::new(rate, map, pitch, length, hooks)?);
                }
                let retimer = state
                    .retimers
                    .get_mut(&item.id)
                    .ok_or(Error::Invalid("retime state"))?;
                let reverse = item.retime.reverse;
                let nested = &mut state.nested;
                let mut pull = |at: i64, count: usize| -> Result<Vec<Vec<f32>>> {
                    if !reverse {
                        return Ok(source_audio(
                            level,
                            item,
                            asset,
                            origin + at,
                            count,
                            sources,
                            nested,
                        )?
                        .planes);
                    }
                    // Reversed sample `n` is window sample `length - 1 - n`: the block
                    // `[at, at + count)` is window `[length - at - count, length - at)`, reversed.
                    let first = length
                        .checked_sub(at)
                        .and_then(|t| t.checked_sub(count as i64))
                        .ok_or(Error::Invalid("reversed source position"))?;
                    let mut planes =
                        source_audio(level, item, asset, origin + first, count, sources, nested)?
                            .planes;
                    for plane in &mut planes {
                        plane.reverse();
                    }
                    Ok(planes)
                };
                let planes = retimer.read(lo - output_origin, count, &mut pull, hooks)?;
                let mut block = AudioBlock::new(
                    rate,
                    u16::try_from(planes.len())
                        .map_err(|_| Error::Invalid("source audio block"))?,
                    count,
                    MemoryKind::Decoded,
                    hooks,
                )?;
                for (dst, src) in block.planes.iter_mut().zip(&planes) {
                    dst.copy_from_slice(src);
                }
                block
            };
            if input.rate != rate
                || input.samples() != count
                || !matches!(input.planes.len(), 1 | 2)
                || input
                    .planes
                    .iter()
                    .any(|p| p.iter().any(|s| !s.is_finite()))
            {
                return Err(Error::Invalid("source audio block"));
            }
            Ok(input)
        };
        // A chain that delays its signal reads `latency` samples ahead of the window, so its
        // output lands on the requested samples: preview, export and any block partition agree
        // and the item stays aligned with its picture and every other item.
        let cleanup_chain = match cleanup_of(item).filter(|cleanup| !cleanup.is_bypassed()) {
            Some(cleanup) => {
                if let std::collections::btree_map::Entry::Vacant(slot) =
                    state.cleanups.entry(item.id)
                {
                    slot.insert(CleanupChain::new(cleanup, rate, hooks)?);
                }
                state.cleanups.get_mut(&item.id)
            }
            None => None,
        };
        let input = match cleanup_chain {
            None => read(a, b)?,
            Some(item_chain) => {
                let ahead = item_chain.latency() as i64;
                if !item_chain.continues(a) {
                    item_chain.restart(a);
                    if ahead > 0 {
                        let mut lead =
                            source_span(item_chain.channels(), &mut read, a, a + ahead, i1)?;
                        item_chain.push(&mut lead);
                    }
                }
                let next = item_chain.fed();
                let mut planes =
                    source_span(item_chain.channels(), &mut read, next, b + ahead, i1)?;
                item_chain.push(&mut planes);
                if planes.iter().any(|plane| plane.len() != (b - a) as usize) {
                    return Err(Error::Invalid("cleanup block"));
                }
                let mut block = AudioBlock::new(
                    rate,
                    u16::try_from(planes.len()).map_err(|_| Error::Invalid("cleanup block"))?,
                    (b - a) as usize,
                    MemoryKind::Decoded,
                    hooks,
                )?;
                for (dst, src) in block.planes.iter_mut().zip(&planes) {
                    dst.copy_from_slice(src);
                }
                block
            }
        };
        let volume_path = project::AnimationPath::from(project::BuiltinPath::Volume);
        let muted_path = project::AnimationPath::from(project::BuiltinPath::Muted);
        let (volume, muted, fades) = match &item.content {
            ItemParams::Audio(p) => (p.volume, p.muted, p.fades),
            ItemParams::Video(p) => (p.volume, p.muted, p.fades),
            ItemParams::Compound(p) => (p.clip.volume, p.clip.muted, p.clip.fades),
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
            if !muted && !project::volume_is_silent(db) {
                let master = if chain.fader {
                    project.data().settings.master_volume
                } else {
                    0.0
                };
                // The master fader at its floor silences the mix as a clip fader does.
                if !project::volume_is_silent(master) {
                    *gain = 10f32.powf((db + master + chain.clip_db(item.id)) / 20.0);
                }
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
            state.retimers.remove(&item.id);
            state.cleanups.remove(&item.id);
            state.nested.remove(&item.id);
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

/// What one mix level shares with the source reads of its items: the project, the output
/// channel count, the edge fade, the nesting depth (0 for the scene) and the execution hooks.
#[derive(Clone, Copy)]
pub(crate) struct Level<'p> {
    pub(crate) project: &'p Project,
    pub(crate) channels: u16,
    pub(crate) fade_ms: u16,
    pub(crate) depth: usize,
    pub(crate) hooks: &'p dyn ExecutionHooks,
}

/// `count` source samples of `item` from source sample `at`: its asset's audio, or a compound
/// clip's nested mix at that source position.
fn source_audio(
    level: Level<'_>,
    item: &project::Item,
    asset: Option<&media::SourceAsset>,
    at: i64,
    count: usize,
    sources: &mut dyn SourceProvider,
    nested: &mut BTreeMap<ItemId, MixState>,
) -> Result<AudioBlock> {
    let rate = level.project.data().settings.audio_sample_rate;
    match (asset, &item.content) {
        (Some(asset), _) => sources.audio(asset, item.id, at, count, rate, level.hooks),
        (None, ItemParams::Compound(params)) => mix_raw(
            Level {
                depth: level.depth + 1,
                ..level
            },
            &params.timeline,
            at,
            count,
            sources,
            &Chain::nested(),
            nested.entry(item.id).or_default(),
        ),
        (None, _) => Err(Error::Invalid("audio source identity")),
    }
}

/// The item's source samples `[lo, target)`: the real ones up to the item's `end`, then silence.
fn source_span(
    channels: usize,
    read: &mut dyn FnMut(i64, i64) -> Result<AudioBlock>,
    lo: i64,
    target: i64,
    end: i64,
) -> Result<Vec<Vec<f32>>> {
    let hi = target.min(end).max(lo);
    let mut planes = if hi > lo {
        read(lo, hi)?.planes
    } else {
        vec![Vec::new(); channels]
    };
    let silence = usize::try_from(target - hi).map_err(|_| Error::Invalid("sample window"))?;
    for plane in &mut planes {
        plane.resize(plane.len() + silence, 0.0);
    }
    Ok(planes)
}

/// The cleanup chain of an item's source audio; other item kinds have none.
fn cleanup_of(item: &project::Item) -> Option<&project::AudioCleanup> {
    match &item.content {
        ItemParams::Audio(p) => Some(&p.cleanup),
        ItemParams::Video(p) => Some(&p.cleanup),
        ItemParams::Compound(p) => Some(&p.clip.cleanup),
        _ => None,
    }
}

/// Conservative source admission: a held mute with no unmute key needs no IO.
/// Any possible unmute keeps the source admitted, including before/after the window.
pub(crate) fn has_source_audio(item: &project::Item) -> bool {
    let muted = match &item.content {
        ItemParams::Video(p) if p.source_audio_enabled => p.muted,
        ItemParams::Compound(p) if p.clip.source_audio_enabled => p.clip.muted,
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

/// The item's output-to-source sample map: its constant ratio, or its speed
/// curve's keys converted to the project sample grid (keys that collapse onto
/// the previous one at this rate are dropped).
fn rate_map(item: &project::Item, rate: u32) -> Result<RateMap> {
    let Some(curve) = &item.retime.curve else {
        return Ok(RateMap::Constant(item.retime.speed.ratio()));
    };
    let mut knots: Vec<(i64, i64)> = Vec::new();
    for key in curve.keys() {
        let knot = (
            key.output.to_units(i64::from(rate), Rounding::Nearest)?,
            key.source.to_units(i64::from(rate), Rounding::Nearest)?,
        );
        if knots
            .last()
            .is_none_or(|last| knot.0 > last.0 && knot.1 > last.1)
        {
            knots.push(knot);
        }
    }
    Ok(RateMap::Curve(knots))
}

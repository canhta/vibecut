use crate::{
    Error, ExecutionHooks, MemoryKind, Reservation, Result, SourceProvider, check, vector,
};
use project::{ItemParams, Project};
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
    limiter: crate::limiter::Limiter,
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
            limiter: crate::limiter::Limiter::new(rate, channels),
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
                self.limiter = crate::limiter::Limiter::new(
                    self.project.data().settings.audio_sample_rate,
                    self.channels,
                );
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
        )?;
        self.limiter.process(&mut out.planes);
        Ok(out)
    }
}

#[allow(clippy::too_many_arguments)]
fn mix_raw(
    project: &Project,
    start: i64,
    frames: usize,
    channels: u16,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    fade_ms: u16,
) -> Result<AudioBlock> {
    check(hooks)?;
    let rate = project.data().settings.audio_sample_rate;
    let end = start
        .checked_add(i64::try_from(frames).map_err(|_| Error::Invalid("sample count"))?)
        .ok_or(Error::Invalid("sample window"))?;
    let mut out = AudioBlock::new(rate, channels, frames, MemoryKind::Compositor, hooks)?;
    if project.data().settings.master_muted {
        return Ok(out);
    }
    let mut spans = Vec::new();
    for track in project.data().scene().timeline.iter() {
        if track.muted {
            continue;
        }
        for item in &track.items {
            if !has_source_audio(item) {
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
    for (item, asset, i0, i1, a, b) in spans {
        check(hooks)?;
        let source = item
            .source_range
            .start()
            .to_units(i64::from(rate), Rounding::Nearest)?
            .checked_add(a - i0)
            .ok_or(Error::Invalid("source sample position"))?;
        let input = sources.audio(asset, item.id, source, (b - a) as usize, rate, hooks)?;
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
        let volume_path = project::AnimationPath::new("volume")?;
        let muted_path = project::AnimationPath::new("muted")?;
        let (volume, muted, fades) = match &item.content {
            ItemParams::Audio(p) => (p.volume, p.muted, p.fades),
            ItemParams::Video(p) => (p.volume, p.muted, p.fades),
            _ => (0.0, true, project::Fades::default()),
        };
        // Clip fades multiply the volume curve before summing and the master limiter.
        let fade_spans = fades.spans(i1 - i0, i64::from(rate))?;
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
            if !muted && !project.data().settings.master_muted {
                *gain = 10f32.powf((db + project.data().settings.master_volume) / 20.0);
            }
        }
        let offset = (a - start) as usize;
        let fade = (i64::from(rate) * i64::from(fade_ms) / 1000).min((i1 - i0) / 2);
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
                    (sample.min(i1 - i0 - 1 - sample) as f32 / fade as f32).clamp(0.0, 1.0)
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
        }
    }
    Ok(out)
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
        .find(|(path, _)| path.as_str() == "muted")
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

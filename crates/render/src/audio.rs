use crate::{
    Error, ExecutionHooks, MemoryKind, Reservation, Result, SourceProvider, check, vector,
};
use project::{ItemParams, Project};
use time::Rounding;
/// Planar f32 accumulation. Values can exceed unity (no implicit limiter).
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
    check(hooks)?;
    let rate = project.data().settings.audio_sample_rate;
    let end = start
        .checked_add(i64::try_from(frames).map_err(|_| Error::Invalid("sample count"))?)
        .ok_or(Error::Invalid("sample window"))?;
    let mut out = AudioBlock::new(rate, channels, frames, MemoryKind::Compositor, hooks)?;
    let mut spans = Vec::new();
    for track in project.data().timeline.iter() {
        if track.muted {
            continue;
        }
        for item in &track.items {
            let enabled = match &item.content {
                ItemParams::Video(p) => p.source_audio_enabled,
                ItemParams::Audio(p) => !p.muted,
                _ => false,
            };
            if !enabled {
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
        let gain = match &item.content {
            ItemParams::Audio(p) => 10f32.powf(p.gain_db / 20.0),
            _ => 1.0,
        };
        let offset = (a - start) as usize;
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
                *dst += value * gain;
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

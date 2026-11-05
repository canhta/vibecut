//! EBU R128 / BS.1770-4 integrated loudness and true-peak measurement, plus the
//! two-pass planner that derives clip and master gains from the shared mix graph.
use crate::{
    Error, ExecutionHooks, MemoryKind, Result, SourceProvider, Stage,
    audio::Chain,
    check,
    oversample::{Interpolator, PolyphaseBank, true_peak_factor},
};
use project::{ItemId, LoudnessGains, Project};
use std::f64::consts::PI;
use time::Rounding;

/// Block energies kept for gating: 27 hours of 100 ms hops.
const MAX_BLOCKS: usize = 1_000_000;
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
const RELATIVE_GATE_LU: f64 = 10.0;
/// Gains within this of unity are stored as exact unity so identity is bit-exact.
const IDENTITY_DB: f64 = 0.05;
/// The limiter's threshold: gain never pushes a true peak above it.
const LIMITER_THRESHOLD_DB: f64 = -1.0;
const WINDOW: usize = 65536;

/// Integrated loudness and true peak of one signal; `None` for silence (no
/// block above the absolute gate, or no sample at all).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub integrated_lufs: Option<f64>,
    pub true_peak_db: Option<f64>,
}

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl Biquad {
    fn step(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// K-weighted (BS.1770-4) gated meter with 4x/2x oversampled true peak.
pub(crate) struct Meter {
    filters: Vec<[Biquad; 2]>,
    bank: PolyphaseBank,
    interp: Vec<Interpolator>,
    hop: usize,
    filled: usize,
    energy: f64,
    recent: [f64; 4],
    seen: usize,
    blocks: Vec<f64>,
    peak: f32,
}
impl Meter {
    pub(crate) fn new(rate: u32, channels: usize) -> Self {
        let fs = f64::from(rate);
        // Shelving stage (head-related), then the RLB high-pass; coefficients
        // follow the BS.1770-4 analogue prototype for any sample rate.
        let (f0, g, q) = (
            1_681.974_450_955_533,
            3.999_843_853_973_347,
            0.707_175_236_955_419_6,
        );
        let k = (PI * f0 / fs).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad {
            b: [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        };
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (PI * f0 / fs).tan();
        let a0 = 1.0 + k / q + k * k;
        let high = Biquad {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        };
        Self {
            filters: vec![[shelf, high]; channels],
            bank: PolyphaseBank::new(true_peak_factor(fs).max(2)),
            interp: vec![Interpolator::default(); channels],
            hop: (rate as usize + 5) / 10,
            filled: 0,
            energy: 0.0,
            recent: [0.0; 4],
            seen: 0,
            blocks: Vec::new(),
            peak: 0.0,
        }
    }

    pub(crate) fn push(&mut self, planes: &[Vec<f32>]) -> Result<()> {
        let frames = planes.first().map_or(0, Vec::len);
        for n in 0..frames {
            for ((plane, stages), interp) in
                planes.iter().zip(&mut self.filters).zip(&mut self.interp)
            {
                let x = plane.get(n).copied().unwrap_or(0.0);
                self.peak = self.peak.max(interp.push_peak(&self.bank, x));
                let [shelf, high] = stages;
                let y = high.step(shelf.step(f64::from(x)));
                self.energy += y * y;
            }
            self.filled += 1;
            if self.filled == self.hop {
                let sub = self.energy / self.hop as f64;
                self.energy = 0.0;
                self.filled = 0;
                self.recent.rotate_left(1);
                if let Some(last) = self.recent.last_mut() {
                    *last = sub;
                }
                self.seen += 1;
                if self.seen >= 4 {
                    if self.blocks.len() >= MAX_BLOCKS {
                        return Err(Error::Invalid("loudness duration"));
                    }
                    self.blocks.push(self.recent.iter().sum::<f64>() / 4.0);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Measurement {
        let lufs = |energy: f64| -0.691 + 10.0 * energy.log10();
        let absolute: Vec<f64> = self
            .blocks
            .iter()
            .copied()
            .filter(|&e| e > 0.0 && lufs(e) > ABSOLUTE_GATE_LUFS)
            .collect();
        let integrated_lufs = if absolute.is_empty() {
            None
        } else {
            let threshold =
                lufs(absolute.iter().sum::<f64>() / absolute.len() as f64) - RELATIVE_GATE_LU;
            let kept: Vec<f64> = absolute
                .iter()
                .copied()
                .filter(|&e| lufs(e) > threshold)
                .collect();
            (!kept.is_empty()).then(|| lufs(kept.iter().sum::<f64>() / kept.len() as f64))
        };
        Measurement {
            integrated_lufs,
            true_peak_db: (self.peak > 0.0).then(|| 20.0 * f64::from(self.peak).log10()),
        }
    }
}

/// Gain that brings `m` to the target without crossing the target's true-peak
/// ceiling or `ceiling_db`; `None` for silence.
fn gain_db(m: Measurement, target: project::LoudnessTarget, ceiling_db: f64) -> Option<f64> {
    let (lufs, peak) = (m.integrated_lufs?, m.true_peak_db?);
    let gain = (f64::from(target.target_lufs) - lufs)
        .min(f64::from(target.true_peak_db).min(ceiling_db) - peak)
        .clamp(
            f64::from(*LoudnessGains::GAIN_RANGE_DB.start()),
            f64::from(*LoudnessGains::GAIN_RANGE_DB.end()),
        );
    Some(if gain.abs() < IDENTITY_DB {
        0.0
    } else {
        // Stored on the same grid as the serialized f32 so round trips are exact.
        f64::from(gain as f32)
    })
}

fn measure_window(
    project: &Project,
    chain: &Chain<'_>,
    start: i64,
    end: i64,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
    progress: &mut dyn FnMut(f32) -> Result<()>,
) -> Result<Measurement> {
    let rate = project.data().settings.audio_sample_rate;
    let _blocks = hooks.reserve(MemoryKind::Compositor, 8 << 20)?;
    let mut meter = Meter::new(rate, 2);
    let mut at = start;
    while at < end {
        check(hooks)?;
        let count = (end - at).min(WINDOW as i64) as usize;
        let block = crate::audio::mix_raw(project, at, count, 2, sources, hooks, 0, chain)?;
        meter.push(block.planes())?;
        at += count as i64;
        progress(((at - start) as f64 / (end - start).max(1) as f64) as f32)?;
    }
    Ok(meter.finish())
}

/// Pass-one measurements and the gains derived from them. `master` is measured
/// with the clip gains already applied.
#[derive(Clone, Debug, PartialEq)]
pub struct LoudnessReport {
    pub gains: LoudnessGains,
    pub clips: Vec<(ItemId, Measurement)>,
    pub master: Option<Measurement>,
}

/// Two-pass planner over the production mix graph. Pass one measures every
/// audible clip alone (gain 0, master fader ignored) when a clip target is set;
/// pass two measures the whole mix with those clip gains applied when a master
/// target is set, so material is never normalized twice. `key` is left zero for
/// the engine, which owns content binding and caching.
pub fn measure_loudness(
    project: &Project,
    sources: &mut dyn SourceProvider,
    hooks: &dyn ExecutionHooks,
) -> Result<LoudnessReport> {
    let settings = &project.data().settings.loudness;
    let rate = i64::from(project.data().settings.audio_sample_rate);
    let end = project.duration().to_units(rate, Rounding::Nearest)?;
    let passes = u32::from(settings.clip.is_some()) + u32::from(settings.master.is_some());
    let mut clip: Vec<(ItemId, f32)> = Vec::new();
    let mut clips = Vec::new();
    let mut done = 0u32;
    if let Some(target) = settings.clip {
        let items = audible_items(project, rate)?;
        let total = items.len().max(1);
        for (index, (id, i0, i1)) in items.iter().enumerate() {
            let chain = Chain::measuring(&[], Some(*id));
            let m = measure_window(
                project,
                &chain,
                (*i0).max(0),
                (*i1).min(end),
                sources,
                hooks,
                &mut |f| {
                    hooks.progress(
                        Stage::Mix,
                        (index as f32 + f) / total as f32 / passes.max(1) as f32,
                    )
                },
            )?;
            clips.push((*id, m));
            // Per-clip normalization has no master chain after it in the target
            // definition, so only the target's own ceiling bounds the gain.
            if let Some(g) = gain_db(m, target, f64::INFINITY).filter(|g| *g != 0.0) {
                clip.push((*id, g as f32));
            }
        }
        clip.sort_by_key(|(id, _)| *id);
        done += 1;
    }
    let mut master_db = 0.0f32;
    let mut master = None;
    if let Some(target) = settings.master {
        let chain = Chain::measuring(&clip, None);
        let m = measure_window(project, &chain, 0, end, sources, hooks, &mut |f| {
            hooks.progress(Stage::Mix, (done as f32 + f) / passes.max(1) as f32)
        })?;
        master = Some(m);
        // The limiter engages above its threshold, which therefore also bounds
        // the gain applied ahead of it. Its fixed 0.98 post gain stays part of
        // the master chain, so already-normalized material is exact identity.
        if let Some(g) = gain_db(m, target, LIMITER_THRESHOLD_DB) {
            master_db = g as f32;
        }
    }
    hooks.progress(Stage::Mix, 1.0)?;
    let gains = LoudnessGains {
        key: [0; 32],
        clip,
        master_db,
    };
    if !gains.is_valid() {
        return Err(Error::Invalid("loudness gains"));
    }
    Ok(LoudnessReport {
        gains,
        clips,
        master,
    })
}

/// Audible source items (`has_source_audio` and an audio stream) with their
/// output sample spans, in timeline order.
fn audible_items(project: &Project, rate: i64) -> Result<Vec<(ItemId, i64, i64)>> {
    let mut items = Vec::new();
    for track in project.data().scene().timeline.iter() {
        if track.muted {
            continue;
        }
        for item in &track.items {
            if !crate::audio::has_source_audio(item) {
                continue;
            }
            let has_stream = item.source_asset.is_some_and(|id| {
                project.data().assets.iter().any(|a| {
                    a.id() == id
                        && a.probe()
                            .streams
                            .iter()
                            .any(|s| s.kind == media::StreamKind::Audio)
                })
            });
            if !has_stream {
                continue;
            }
            if items.len() >= LoudnessGains::MAX_CLIPS {
                return Err(Error::Invalid("audio window expansion"));
            }
            items.push((
                item.id,
                item.start.to_units(rate, Rounding::Nearest)?,
                item.end()?.to_units(rate, Rounding::Nearest)?,
            ));
        }
    }
    Ok(items)
}

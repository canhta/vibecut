#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use crate::support::*;
use project::{AnimationPath, Channel, Extrapolation, Keyframe, Param, ParamValue};
use render::*;
use time::{Tick, TimeRange};

struct Signal {
    level: f32,
    impulse: bool,
}

#[test]
fn viewport_audio_only_project_has_no_visual_selection() {
    let project = fixture();
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let viewport = render::viewport::Viewport::new(
        Rect {
            x: 0.,
            y: 0.,
            width: 1920.,
            height: 1080.,
        },
        [1920, 1080],
        1.,
        [0., 0.],
    )
    .unwrap();
    assert!(
        renderer
            .item_geometry(project::ItemId(1), Tick::ZERO)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        renderer
            .hit_test(&viewport, [960., 540.], Tick::ZERO)
            .unwrap(),
        None
    );
}
impl SourceProvider for Signal {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                *sample = if !self.impulse || (start + n as i64).rem_euclid(503) == 0 {
                    self.level
                } else {
                    0.0
                };
            }
        }
        Ok(out)
    }
}
fn fixture() -> project::Project {
    fixture_of(1)
}
fn fixture_of(seconds: i64) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(1), "".into())
        .unwrap()
        .into_data();
    let asset = testkit::source::asset(1, std::env::temp_dir().join("mixer.wav"))
        .format("wav")
        .duration(tick(seconds, 1))
        .content_hash(media::ContentHash::from_bytes(&[42]))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(tick(seconds, 1))
                .build(),
        )
        .build();
    data.assets.push(asset);
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![
            testkit::item::audio(project::ItemId(1))
                .asset(media::AssetId(1))
                .source(TimeRange::new(Tick::ZERO, tick(seconds, 1)).unwrap())
                .build(),
        ],
    });
    project::Project::new(data).unwrap()
}
#[test]
fn limiter_ceiling_transparency_latency_and_irregular_partitions() {
    let h = Hooks::default();
    for (level, impulse, master) in [
        (0.1, false, 0.0),
        (0.1, false, -6.0),
        (0.1, false, 20.0),
        (8.0, false, 0.0),
        (100.0, true, 0.0),
        (-8.0, true, 0.0),
    ] {
        let mut data = fixture().into_data();
        data.settings.master_volume = master;
        let p = project::Project::new(data).unwrap();
        let mut sources = Signal { level, impulse };
        let mut offline = AudioMixer::new(&p, 2, 0, &h).unwrap();
        assert_eq!(offline.latency_samples(), 248);
        let whole = offline.mix(0, 48000, &mut sources, &h).unwrap();
        assert!(whole.peak() <= 10f32.powf(-1.0 / 20.0) * 0.98);
        if level == 0.1 && master <= 0.0 {
            let expected = level * 10f32.powf(master / 20.0) * 0.98;
            assert!(whole.planes().iter().flatten().all(|&v| v == expected));
        }
        if impulse {
            assert!(whole.planes()[0][0].abs() > 0.01);
            assert_eq!(whole.planes()[0][1], 0.0);
        }
        let mut realtime = AudioMixer::new(&p, 2, 0, &h).unwrap();
        let mut joined = vec![vec![], vec![]];
        let mut at = 0;
        let mut index = 0;
        while at < 48000 {
            let frames = [1, 7, 1031, 4093, 5, 23999][index % 6].min(48000 - at);
            let block = realtime.mix(at as i64, frames, &mut sources, &h).unwrap();
            for (dst, src) in joined.iter_mut().zip(block.planes()) {
                dst.extend_from_slice(src);
            }
            at += frames;
            index += 1;
        }
        assert_eq!(whole.planes(), joined);
        for start in [491, 0, 1783] {
            // Discontinuous seeks reconstruct the same envelope.
            let block = realtime.mix(start, 111, &mut sources, &h).unwrap();
            assert_eq!(
                block.planes()[0],
                whole.planes()[0][start as usize..start as usize + 111]
            );
        }
    }
}
#[test]
fn automation_grid_continues_when_another_track_ends_and_mute_holds() {
    let mut data = fixture().into_data();
    let item = &mut data.scene_mut().timeline.audio[0].items[0];
    item.animations
        .set(
            AnimationPath::new("volume").unwrap(),
            Param::Value(
                Channel::new(
                    ParamValue::Scalar(-20.0),
                    vec![
                        Keyframe::new(Tick::ZERO, ParamValue::Scalar(-20.0)),
                        Keyframe::new(tick(1, 1), ParamValue::Scalar(0.0)),
                    ],
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let mut key = Keyframe::new(tick(1001, 48000), ParamValue::Bool(true));
    key.segment = project::Segment::Step;
    item.animations
        .set(
            AnimationPath::new("muted").unwrap(),
            Param::Value(
                Channel::new(
                    ParamValue::Bool(false),
                    vec![key],
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let mut short = data.scene().timeline.audio[0].clone();
    short.id = project::TrackId(2);
    short.items[0].id = project::ItemId(2);
    short.items[0].animations = Default::default();
    short.items[0].source_range = TimeRange::new(Tick::ZERO, tick(503, 48000)).unwrap();
    data.scene_mut().timeline.insert_track(short);
    let p = project::Project::new(data).unwrap();
    let h = Hooks::default();
    let mut source = Signal {
        level: 0.1,
        impulse: false,
    };
    let whole = mix_audio(&p, 0, 2003, 2, &mut source, &h).unwrap();
    for n in [502, 503, 504, 999, 1000] {
        let expected = (if n < 503 { 0.1 } else { 0.0 })
            + 0.1 * 10f32.powf((-20.0 + 20.0 * n as f32 / 48000.0) / 20.0);
        assert!((whole.planes()[0][n] - expected * 0.98).abs() < 1e-7);
    }
    assert!(whole.planes()[0][1001..].iter().all(|&v| v == 0.0));
    let mut mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
    for start in [0, 502, 503, 1000, 1001] {
        let part = mixer.mix(start, 13, &mut source, &h).unwrap();
        assert_eq!(
            part.planes()[0],
            whole.planes()[0][start as usize..start as usize + 13]
        );
    }
}
#[test]
fn empty_muted_and_outside_timeline_are_exact_silence_and_failures_reset_state() {
    let h = Hooks::default();
    let mut source = Signal {
        level: 0.1,
        impulse: false,
    };
    for p in [
        project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
        {
            let mut d = fixture().into_data();
            d.settings.master_muted = true;
            project::Project::new(d).unwrap()
        },
        {
            let mut d = fixture().into_data();
            d.scene_mut().timeline.audio[0].items[0]
                .animations
                .set(
                    AnimationPath::new("muted").unwrap(),
                    Param::new(ParamValue::Bool(true)).unwrap(),
                )
                .unwrap();
            project::Project::new(d).unwrap()
        },
    ] {
        // NaN would refuse if a silent source were unnecessarily decoded.
        let mut unavailable = Signal {
            level: f32::NAN,
            impulse: false,
        };
        let block = mix_audio(&p, -5, 2003, 2, &mut unavailable, &h).unwrap();
        assert!(block.planes().iter().flatten().all(|&v| v.to_bits() == 0));
    }
    let p = fixture();
    let mut mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
    assert!(mixer.mix(i64::MAX, 2, &mut source, &h).is_err());
    assert!(mixer.mix(0, 65537, &mut source, &h).is_err());
    assert_eq!(mixer.mix(48000, 13, &mut source, &h).unwrap().peak(), 0.0);
    h.state.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        mixer.mix(0, 13, &mut source, &h),
        Err(Error::Cancelled)
    ));
    h.state.store(0, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        mixer.mix(0, 13, &mut source, &h).unwrap().planes()[0],
        [0.1 * 0.98; 13]
    );
    let mut invalid = Signal {
        level: f32::NAN,
        impulse: false,
    };
    assert!(mixer.mix(13, 13, &mut invalid, &h).is_err());
    assert_eq!(
        mixer.mix(13, 13, &mut source, &h).unwrap().planes()[0],
        [0.1 * 0.98; 13]
    );
}

fn faded(fades: project::Fades) -> project::Project {
    let mut data = fixture().into_data();
    if let project::ItemParams::Audio(p) = &mut data.scene_mut().timeline.audio[0].items[0].content
    {
        p.fades = fades;
    }
    project::Project::new(data).unwrap()
}
fn fade(ms: i64, curve: project::FadeCurve) -> project::Fade {
    project::Fade {
        duration: tick(ms, 1000),
        offset: Tick::ZERO,
        curve,
    }
}
fn mix_all(p: &project::Project, partition: &[usize]) -> Vec<f32> {
    let h = Hooks::default();
    let mut sources = Signal {
        level: 0.1,
        impulse: false,
    };
    let mut mixer = AudioMixer::new(p, 2, 0, &h).unwrap();
    let mut out = Vec::new();
    let (mut at, mut index) = (0usize, 0usize);
    while at < 48000 {
        let frames = partition[index % partition.len()].min(48000 - at);
        let block = mixer.mix(at as i64, frames, &mut sources, &h).unwrap();
        out.extend_from_slice(&block.planes()[0]);
        at += frames;
        index += 1;
    }
    out
}

#[test]
fn a_split_clip_mixes_to_the_same_samples_as_the_whole_clip() {
    let fades = project::Fades {
        fade_in: fade(300, project::FadeCurve::Exponential),
        fade_out: fade(400, project::FadeCurve::EqualPower),
    };
    let whole = mix_all(&faded(fades), &[4096]);
    // Cuts inside the fade-in, between fades, inside the fade-out; off the sample grid.
    for cut_ms in [101, 300, 550, 700, 899] {
        let at = Tick(tick(cut_ms, 1000).0 + 12_345);
        let mut data = faded(fades).into_data();
        let items = &mut data.scene_mut().timeline.audio[0].items;
        let (mut left, mut right) = (items[0].clone(), items[0].clone());
        let end = left.end().unwrap();
        let (l, r) = fades.split(Tick::ZERO, at, end).unwrap();
        left.source_range = TimeRange::new(Tick::ZERO, at).unwrap();
        right.id = project::ItemId(2);
        right.start = at;
        right.source_range = TimeRange::new(at, end).unwrap();
        let set = |item: &mut project::Item, f| {
            if let project::ItemParams::Audio(p) = &mut item.content {
                p.fades = f;
            }
        };
        set(&mut left, l);
        set(&mut right, r);
        *items = vec![left, right];
        let split = project::Project::new(data).unwrap();
        for partition in [&[4096][..], &[1, 7, 1031, 4093]] {
            assert_eq!(whole, mix_all(&split, partition), "cut {cut_ms} ms");
        }
    }
}

#[test]
fn clip_fades_follow_the_shared_gain_exactly_and_do_not_depend_on_blocks() {
    // Unity source through the limiter's constant 0.98 passthrough: output = 0.098 * gain.
    let unit = 0.1f32 * 0.98;
    for (fade_in, fade_out) in [
        (
            fade(100, project::FadeCurve::EqualPower),
            fade(200, project::FadeCurve::Linear),
        ),
        (
            fade(50, project::FadeCurve::Exponential),
            fade(50, project::FadeCurve::EqualPower),
        ),
        // Oversized requests clamp to the clip; overlapping windows multiply.
        (
            fade(800, project::FadeCurve::Linear),
            fade(800, project::FadeCurve::Linear),
        ),
    ] {
        let fades = project::Fades { fade_in, fade_out };
        let p = faded(fades);
        let spans = fades.spans(Tick::ZERO, tick(1000, 1000), 48000).unwrap();
        let (i, o) = spans.lengths();
        assert!(i <= 48000 && o <= 48000);
        let whole = mix_all(&p, &[48000]);
        for (n, sample) in whole.iter().enumerate() {
            let expected = unit * spans.gain(n as i64);
            assert!(
                (sample - expected).abs() <= 1e-7,
                "sample {n}: {sample} vs {expected}"
            );
        }
        // Clip edges are exactly silent and the body is exactly unity.
        assert_eq!(whole[0], 0.0);
        assert_eq!(whole[47999], 0.0);
        if i > 0 && o > 0 && i + o < 48000 {
            assert_eq!(whole[(i) as usize], unit);
        }
        // Preview-style and export-style partitions are bit identical.
        assert_eq!(whole, mix_all(&p, &[1, 7, 1031, 4093, 5, 23999]));
        assert_eq!(whole, mix_all(&p, &[2048]));
        // No click: adjacent steps stay within the steepest curve slope (exponential, 4.07/len).
        let shortest = i.min(o).max(1) as f32;
        let bound = unit * 4.1 / shortest + 1e-7;
        let max_step = whole
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step <= bound, "step {max_step} > {bound}");
    }
}

#[test]
fn clip_fades_compose_with_volume_keyframes_and_the_master_limiter() {
    let fades = project::Fades {
        fade_in: fade(100, project::FadeCurve::Linear),
        fade_out: fade(100, project::FadeCurve::Linear),
    };
    let mut data = faded(fades).into_data();
    data.settings.master_volume = -6.0;
    data.scene_mut().timeline.audio[0].items[0]
        .animations
        .set(
            AnimationPath::new("volume").unwrap(),
            Param::Value(
                Channel::new(
                    ParamValue::Scalar(-20.0),
                    vec![
                        Keyframe::new(Tick::ZERO, ParamValue::Scalar(-20.0)),
                        Keyframe::new(tick(1, 1), ParamValue::Scalar(0.0)),
                    ],
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let p = project::Project::new(data).unwrap();
    let mut plain = p.clone().into_data();
    if let project::ItemParams::Audio(p) = &mut plain.scene_mut().timeline.audio[0].items[0].content
    {
        p.fades = Default::default();
    }
    let plain = project::Project::new(plain).unwrap();
    let with = mix_all(&p, &[4096]);
    let without = mix_all(&plain, &[4096]);
    let spans = fades.spans(Tick::ZERO, tick(1000, 1000), 48000).unwrap();
    for n in 0..48000 {
        let expected = without[n] * spans.gain(n as i64);
        assert!((with[n] - expected).abs() <= 1e-7, "sample {n}");
    }
    // Hot source: the limiter still bounds the faded sum.
    let h = Hooks::default();
    let mut hot = Signal {
        level: 100.0,
        impulse: false,
    };
    let block = AudioMixer::new(&p, 2, 0, &h)
        .unwrap()
        .mix(0, 48000, &mut hot, &h)
        .unwrap();
    assert!(block.peak() <= 10f32.powf(-1.0 / 20.0) * 0.98);
    // Cut fades (`fade_ms`) still apply on top of clip fades.
    let cut = AudioMixer::new(&p, 2, 10, &h)
        .unwrap()
        .mix(
            0,
            48000,
            &mut Signal {
                level: 0.1,
                impulse: false,
            },
            &h,
        )
        .unwrap();
    assert!(
        cut.planes()[0][..480]
            .iter()
            .zip(&with[..480])
            .all(|(c, w)| c.abs() <= w.abs() + 1e-7)
    );
}

fn cleaned(effects: Vec<project::CleanupEffect>) -> project::Project {
    let mut data = fixture().into_data();
    if let project::ItemParams::Audio(p) = &mut data.scene_mut().timeline.audio[0].items[0].content
    {
        p.cleanup = project::AudioCleanup { effects };
    }
    project::Project::new(data).unwrap()
}
fn cleanup(id: u32, wet_percent: f32, kind: project::CleanupKind) -> project::CleanupEffect {
    project::CleanupEffect {
        id,
        enabled: true,
        wet_percent,
        kind,
        preset: None,
    }
}

#[test]
fn cleanup_chain_bypass_is_identity_and_filters_do_not_depend_on_blocks() {
    let plain = mix_all(&fixture(), &[4096]);
    let highpass = project::CleanupKind::Highpass { cutoff_hz: 1000.0 };
    // Bypassed: disabled, zero wet, and a zero-gain shelf each leave the source unchanged.
    let mut disabled = cleanup(1, 100.0, highpass.clone());
    disabled.enabled = false;
    let bypassed = cleaned(vec![
        disabled,
        cleanup(2, 0.0, highpass.clone()),
        cleanup(3, 100.0, project::CleanupKind::Bass { boost_db: 0.0 }),
    ]);
    assert_eq!(plain, mix_all(&bypassed, &[4096]));
    // A 1 kHz high-pass removes the constant source after settling and keeps the transient.
    let filtered = cleaned(vec![
        cleanup(4, 100.0, highpass),
        cleanup(
            5,
            100.0,
            project::CleanupKind::ParametricEq {
                bands: vec![project::EqBand {
                    kind: project::EqBandKind::Peaking,
                    enabled: true,
                    frequency_hz: 3000.0,
                    gain_db: 6.0,
                    q: 1.0,
                }],
            },
        ),
    ]);
    let whole = mix_all(&filtered, &[4096]);
    assert!(whole[..10].iter().any(|s| s.abs() > 0.01));
    assert!(whole[40_000..].iter().all(|s| s.abs() < 1e-4));
    // Any partition of the same item gives identical samples.
    for partition in [&[1, 7, 1031, 4093][..], &[5, 23999]] {
        assert_eq!(whole, mix_all(&filtered, partition));
    }
}

#[test]
fn bass_boost_follows_its_dry_wet_mix_on_the_settled_signal() {
    // Constant 0.1 source through the limiter's 0.98 passthrough; a +6 dB low shelf has 2.0 DC gain
    // at full wet and unity at 0 % wet.
    let dry = 0.1f32 * 0.98;
    let boost = 10f32.powf(6.0 / 20.0);
    for (wet_percent, expected) in [
        (100.0, dry * boost),
        (50.0, dry * (0.5 + 0.5 * boost)),
        (0.0, dry),
    ] {
        let p = cleaned(vec![cleanup(
            1,
            wet_percent,
            project::CleanupKind::Bass { boost_db: 6.0 },
        )]);
        let settled = mix_all(&p, &[4096])[40_000];
        assert!(
            (settled - expected).abs() < 1e-3,
            "wet {wet_percent}: {settled} vs {expected}"
        );
    }
}

/// Deterministic synthetic source: white noise, a tone (optionally in half-second bursts) and
/// two-sample clicks, all pure functions of the absolute sample.
struct Synth {
    noise: f32,
    tone: f32,
    hz: f64,
    bursts: bool,
    clicks: bool,
}
fn uniform(n: i64) -> f32 {
    let mut x = (n as u64)
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}
impl Synth {
    fn value(&self, n: i64) -> f32 {
        let on = !self.bursts || (n / 24_000) % 2 == 1;
        let tone = if on {
            self.tone * (2.0 * std::f64::consts::PI * self.hz * n as f64 / 48_000.0).sin() as f32
        } else {
            0.0
        };
        let mut value = self.noise * uniform(n) + tone;
        if self.clicks {
            for k in 0..30 {
                let at = click_at(k);
                if n == at {
                    value += 0.5;
                } else if n == at + 1 {
                    value -= 0.3;
                }
            }
        }
        value
    }
}
fn click_at(k: i64) -> i64 {
    3000 + k * 3001
}
impl SourceProvider for Synth {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                *sample = self.value(start + n as i64);
            }
        }
        Ok(out)
    }
}
fn with_cleanup(base: project::Project, effects: Vec<project::CleanupEffect>) -> project::Project {
    let mut data = base.into_data();
    if let project::ItemParams::Audio(p) = &mut data.scene_mut().timeline.audio[0].items[0].content
    {
        p.cleanup = project::AudioCleanup { effects };
    }
    project::Project::new(data).unwrap()
}
fn noise_reduction(reduction_db: f32) -> project::CleanupKind {
    project::CleanupKind::NoiseReduction {
        reduction_db,
        threshold_db: 6.0,
        smoothing_percent: 50.0,
    }
}
fn declick() -> project::CleanupKind {
    project::CleanupKind::Declick {
        threshold: 30.0,
        complexity: 16.0,
    }
}
/// Left plane of `seconds` of the mix, requested in blocks of the `partition` sizes.
fn mix_synth(
    p: &project::Project,
    source: &mut impl SourceProvider,
    seconds: usize,
    partition: &[usize],
) -> Vec<f32> {
    let h = Hooks::default();
    let total = seconds * 48_000;
    let mut mixer = AudioMixer::new(p, 2, 0, &h).unwrap();
    let mut out = Vec::new();
    let (mut at, mut index) = (0usize, 0usize);
    while at < total {
        let frames = partition[index % partition.len()].min(total - at);
        let block = mixer.mix(at as i64, frames, source, &h).unwrap();
        out.extend_from_slice(&block.planes()[0]);
        at += frames;
        index += 1;
    }
    out
}
fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn noise_reduction_cuts_the_noise_floor_keeps_tones_and_stays_on_the_source_samples() {
    // 1 kHz bursts (0.5 s on, 0.5 s off) over white noise at 0.02, eight seconds.
    let mut synth = Synth {
        noise: 0.02,
        tone: 0.3,
        hz: 1000.0,
        bursts: true,
        clicks: false,
    };
    let plain = mix_synth(&fixture_of(8), &mut synth, 8, &[65536]);
    let reduce =
        |db: f32| with_cleanup(fixture_of(8), vec![cleanup(1, 100.0, noise_reduction(db))]);
    // With the gate open the chain is a delayed identity, and the delay is compensated: every
    // output sample equals the plain mix sample.
    let open = mix_synth(&reduce(0.0), &mut synth, 8, &[65536]);
    let worst = plain
        .iter()
        .zip(&open)
        .map(|(p, o)| (p - o).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-4, "gate-open error {worst}");
    // 20 dB reduction: the noise-only half seconds fall by more than 10 dB, the tone bursts
    // stay within 1 dB, and nothing gets louder than the source.
    let reduced = mix_synth(&reduce(20.0), &mut synth, 8, &[65536]);
    let half = 24_000;
    let segment = |k: usize| k * half + half / 4..k * half + 3 * half / 4;
    let db = |a: &[f32], b: &[f32]| 20.0 * (rms(a) / rms(b)).log10();
    let (off, on) = (segment(14), segment(15));
    let floor = db(&reduced[off.clone()], &plain[off]);
    assert!(floor < -10.0, "noise floor only {floor} dB lower");
    let tone = db(&reduced[on.clone()], &plain[on]);
    assert!(tone.abs() < 1.0, "tone changed by {tone} dB");
    let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak(&reduced) <= peak(&plain) + 1e-3);
}

#[test]
fn delayed_cleanup_reports_its_latency_and_lands_within_one_sample_of_the_source() {
    let h = Hooks::default();
    let tone = || Synth {
        noise: 0.0,
        tone: 0.2,
        hz: 1000.0,
        bursts: false,
        clicks: false,
    };
    let plain = mix_synth(&fixture(), &mut tone(), 1, &[4096]);
    // Reported delay: nothing for filters, one frame for noise reduction, 640 samples for
    // de-click, their sum for both. The limiter's own latency never includes it.
    let bass = project::CleanupKind::Bass { boost_db: 3.0 };
    let cases = [
        (vec![], 0),
        (vec![cleanup(1, 100.0, bass)], 0),
        (vec![cleanup(1, 100.0, noise_reduction(0.0))], 2048),
        (vec![cleanup(1, 100.0, declick())], 640),
        (
            vec![
                cleanup(1, 100.0, noise_reduction(0.0)),
                cleanup(2, 100.0, declick()),
            ],
            2688,
        ),
        // A bypassed delayed effect delays nothing.
        (vec![cleanup(1, 0.0, noise_reduction(20.0))], 0),
    ];
    for (effects, latency) in cases {
        let p = with_cleanup(fixture(), effects.clone());
        let mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
        assert_eq!(mixer.cleanup_latency_samples(), latency, "{effects:?}");
        assert_eq!(mixer.latency_samples(), 248);
        // A one-sample shift of a 1 kHz tone at 0.2 changes it by 0.026, so matching the plain
        // mix to 1e-3 proves alignment far inside one sample. The first frames are skipped: a
        // signal that starts from silence is not a stationary tone.
        let out = mix_synth(&p, &mut tone(), 1, &[4096]);
        let worst = plain
            .iter()
            .zip(&out)
            .skip(2000)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        if latency > 0 {
            assert!(worst < 1e-3, "{effects:?}: off by {worst}");
        }
    }
    // An impulse train is the sharpest alignment probe: with the gate open every impulse lands
    // on the plain mix's sample.
    let mut impulses = Signal {
        level: 0.5,
        impulse: true,
    };
    let mut mix_impulses = |p: &project::Project| {
        let mut mixer = AudioMixer::new(p, 2, 0, &h).unwrap();
        mixer.mix(0, 48000, &mut impulses, &h).unwrap().planes()[0].clone()
    };
    let reference = mix_impulses(&fixture());
    let delayed = mix_impulses(&with_cleanup(
        fixture(),
        vec![cleanup(1, 100.0, noise_reduction(0.0))],
    ));
    assert!(reference.iter().any(|s| s.abs() > 0.4));
    let worst = reference
        .iter()
        .zip(&delayed)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-3, "impulse train off by {worst}");
    // Bypass is bit-identical to no chain.
    let mut off = cleanup(2, 100.0, declick());
    off.enabled = false;
    let bypassed = with_cleanup(fixture(), vec![cleanup(1, 0.0, noise_reduction(20.0)), off]);
    assert_eq!(plain, mix_synth(&bypassed, &mut tone(), 1, &[4096]));
}

#[test]
fn declick_repairs_clicks_and_follows_its_dry_wet_mix() {
    let seconds = 2;
    let source = |clicks| Synth {
        noise: 0.01,
        tone: 0.15,
        hz: 440.0,
        bursts: false,
        clicks,
    };
    let base = fixture_of(2);
    let clean = mix_synth(&base, &mut source(false), seconds, &[65536]);
    let dirty = mix_synth(&base, &mut source(true), seconds, &[65536]);
    let repaired = mix_synth(
        &with_cleanup(fixture_of(2), vec![cleanup(1, 100.0, declick())]),
        &mut source(true),
        seconds,
        &[65536],
    );
    let error = |s: &[f32]| {
        s.iter()
            .zip(&clean)
            .skip(2000)
            .map(|(a, b)| f64::from(a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let (before, after) = (error(&dirty), error(&repaired));
    assert!(
        20.0 * (after / before).log10() < -20.0,
        "click error {before} -> {after}"
    );
    // The mix holds a click at half strength, so the dry signal is delayed with the wet one.
    let half = mix_synth(
        &with_cleanup(fixture_of(2), vec![cleanup(1, 50.0, declick())]),
        &mut source(true),
        seconds,
        &[65536],
    );
    let at = click_at(3) as usize;
    let deviation = |s: &[f32]| (s[at] - clean[at]).abs();
    assert!(deviation(&dirty) > 0.45, "click {}", deviation(&dirty));
    assert!(
        deviation(&repaired) < 0.03,
        "repaired {}",
        deviation(&repaired)
    );
    assert!(
        (deviation(&half) - 0.5 * deviation(&dirty)).abs() < 0.03,
        "half mix {}",
        deviation(&half)
    );
}

#[test]
fn delayed_cleanup_is_identical_across_partitions_seeks_and_offline_export() {
    let seconds = 2;
    let mut source = Synth {
        noise: 0.01,
        tone: 0.15,
        hz: 440.0,
        bursts: false,
        clicks: true,
    };
    let p = with_cleanup(
        fixture_of(2),
        vec![
            cleanup(1, 100.0, noise_reduction(12.0)),
            cleanup(2, 80.0, declick()),
            cleanup(3, 100.0, project::CleanupKind::Highpass { cutoff_hz: 60.0 }),
        ],
    );
    // Offline export pulls large blocks; realtime pulls whatever the device asks for.
    let offline = mix_synth(&p, &mut source, seconds, &[65536]);
    assert!(offline.iter().any(|s| s.abs() > 0.1));
    for partition in [&[1, 7, 1031, 4093][..], &[5, 23999], &[480]] {
        assert_eq!(offline, mix_synth(&p, &mut source, seconds, partition));
    }
    // A seek replays from the start, so a window equals the same window of the whole mix.
    let h = Hooks::default();
    let mut mixer = AudioMixer::new(&p, 2, 0, &h).unwrap();
    for start in [70_000usize, 491, 0, 1783, 3001] {
        let block = mixer.mix(start as i64, 111, &mut source, &h).unwrap();
        assert_eq!(block.planes()[0], offline[start..start + 111], "{start}");
    }
}

/// Deterministic voiced speech stand-in: a 140 Hz harmonic stack to 2.8 kHz in half-second
/// bursts (0.5 s on, 0.5 s off) over white noise at 0.02 and a 45 Hz rumble at 0.03.
struct Voiced;
impl Voiced {
    fn value(n: i64) -> f32 {
        let t = n as f64 / 48_000.0;
        let tau = 2.0 * std::f64::consts::PI;
        let speech = if (n / 24_000) % 2 == 1 {
            (1..=20)
                .map(|h| 0.08 / h as f64 * (tau * 140.0 * h as f64 * t).sin())
                .sum::<f64>() as f32
        } else {
            0.0
        };
        speech + 0.02 * uniform(n) + 0.03 * (tau * 45.0 * t).sin() as f32
    }
}
/// A source that is a pure function of the absolute sample, the same on both channels.
struct Wave(fn(i64) -> f32);
impl SourceProvider for Wave {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            for (n, sample) in plane.iter_mut().enumerate() {
                *sample = (self.0)(start + n as i64);
            }
        }
        Ok(out)
    }
}

#[test]
fn voice_enhancement_preset_lifts_speech_over_noise_without_pumping_and_bypass_is_identity() {
    // Metric, fixed before the chain was written: speech-to-noise ratio in dB, the RMS of the
    // middle halves of the speech-on half seconds over the RMS of the middle halves of the
    // speech-off ones, on the eight second fixture above. Pass: the preset raises it by at
    // least 6 dB (the noise reduction stage alone is designed for 12), moves the speech level
    // by less than 3 dB, holds the speech-on gain change across 50 ms windows within 3 dB (no
    // pumping), peaks below 0.9, and leaves the sample grid aligned (latency is the two
    // spectral frames only, de-reverb and noise reduction).
    let seconds = 8;
    let half = 24_000;
    let mix = |p: &project::Project, partition: &[usize]| {
        mix_synth(p, &mut Wave(Voiced::value), seconds, partition)
    };
    let plain = mix(&fixture_of(8), &[65536]);
    let preset = project::voice_enhancement_effects(1).to_vec();
    let enhanced_project = with_cleanup(fixture_of(8), preset.clone());
    let enhanced = mix(&enhanced_project, &[65536]);
    let middle = |k: usize| k * half + half / 4..k * half + 3 * half / 4;
    let gather = |signal: &[f32], ks: &[usize]| -> Vec<f32> {
        ks.iter()
            .flat_map(|k| signal[middle(*k)].iter().copied())
            .collect()
    };
    let (speech, noise) = ([13usize, 15], [12usize, 14]);
    let ratio = |signal: &[f32]| {
        20.0 * (rms(&gather(signal, &speech)) / rms(&gather(signal, &noise))).log10()
    };
    let gain = ratio(&enhanced) - ratio(&plain);
    assert!(gain >= 6.0, "speech-to-noise ratio only rose {gain} dB");
    let level = 20.0
        * (rms(&gather(&enhanced, &speech)) / rms(&gather(&plain, &speech))).log10();
    assert!(level.abs() < 3.0, "speech level moved {level} dB");
    let peak = enhanced.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak < 0.9, "peak {peak}");
    // No pumping: the enhanced-to-plain level ratio stays steady across 50 ms windows of speech.
    let window = 2_400;
    let mut ratios = Vec::new();
    for k in speech {
        for w in 0..(half / 2) / window {
            let at = middle(k).start + w * window;
            let (e, p) = (&enhanced[at..at + window], &plain[at..at + window]);
            ratios.push(20.0 * (rms(e) / rms(p)).log10());
        }
    }
    let spread = ratios.iter().fold(f32::MIN, |m, r| m.max(*r))
        - ratios.iter().fold(f32::MAX, |m, r| m.min(*r));
    assert!(spread < 3.0, "gain changes {spread} dB across speech");
    // Preview and export agree for any block partition, and the chain's only delay is the
    // de-reverb and noise reduction frames, which the mix graph compensates.
    assert_eq!(enhanced, mix(&enhanced_project, &[1, 7, 1031, 4093]));
    let h = Hooks::default();
    let mixer = AudioMixer::new(&enhanced_project, 2, 0, &h).unwrap();
    assert_eq!(mixer.cleanup_latency_samples(), 4096);
    // Bypass: every preset effect disabled, or at zero mix, is bit-identical to no chain.
    let off = preset
        .iter()
        .cloned()
        .map(|mut effect| {
            effect.enabled = false;
            effect
        })
        .collect();
    assert_eq!(plain, mix(&with_cleanup(fixture_of(8), off), &[65536]));
    let dry = preset
        .iter()
        .cloned()
        .map(|mut effect| {
            effect.wet_percent = 0.0;
            effect
        })
        .collect();
    assert_eq!(plain, mix(&with_cleanup(fixture_of(8), dry), &[65536]));
}

fn voiced_stack(n: i64) -> f32 {
    let t = n as f64 / 48_000.0;
    (1..=20)
        .map(|h| 0.08 / h as f64 * (2.0 * std::f64::consts::PI * 140.0 * h as f64 * t).sin())
        .sum::<f64>() as f32
}
fn sibilant_band(n: i64) -> f32 {
    let t = n as f64 / 48_000.0;
    let tone = |hz: f64| (2.0 * std::f64::consts::PI * hz * t).sin();
    (0.05 * (tone(5_500.0) + tone(7_000.0) + tone(8_500.0))) as f32
}
/// White noise whose amplitude decays by 60 dB in 0.8 s: a diffuse reverberant tail of that
/// RT60, starting at full level at sample zero, silent after two seconds.
fn decaying_tail(n: i64) -> f32 {
    let t = n as f32 / 48_000.0;
    if t < 2.0 {
        0.3 * uniform(n) * 10f32.powf(-3.0 * t / 0.8)
    } else {
        0.0
    }
}
fn db_between(a: &[f32], b: &[f32]) -> f32 {
    20.0 * (rms(a) / rms(b)).log10()
}
/// Mixes two seconds of `wave` through `effects` on one audio item.
fn mix_wave(wave: fn(i64) -> f32, effects: Vec<project::CleanupEffect>) -> Vec<f32> {
    mix_synth(
        &with_cleanup(fixture_of(2), effects),
        &mut Wave(wave),
        2,
        &[4096],
    )
}
/// Bypass identity of one effect: disabled or at zero mix it is bit-identical to no chain, and
/// with the neutral parameters it is the plain mix within 1e-4 (the delay compensated).
fn assert_bypass_is_identity(
    wave: fn(i64) -> f32,
    active: project::CleanupKind,
    neutral: project::CleanupKind,
) {
    let plain = mix_wave(wave, vec![]);
    let mut off = cleanup(1, 100.0, active.clone());
    off.enabled = false;
    assert_eq!(plain, mix_wave(wave, vec![off]));
    assert_eq!(plain, mix_wave(wave, vec![cleanup(1, 0.0, active)]));
    let open = mix_wave(wave, vec![cleanup(1, 100.0, neutral.clone())]);
    let worst = plain
        .iter()
        .zip(&open)
        .map(|(p, o)| (p - o).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-4, "{neutral:?} neutral error {worst}");
}

#[test]
fn deesser_cuts_a_sibilant_band_leaves_the_voiced_stack_and_bypass_is_identity() {
    // Stated limits at 6 kHz, threshold -12 dB, reduction 8 dB: a steady 5.5, 7 and 8.5 kHz
    // band falls by at least 4 dB (a shelf at 6 kHz cuts 5.5 kHz by about 3 dB and 8.5 kHz by
    // about 7 dB), and the 140 Hz harmonic stack to 2.8 kHz moves by less than 1 dB.
    let deess = |reduction_db: f32| project::CleanupKind::Deess {
        frequency_hz: 6000.0,
        threshold_db: -12.0,
        reduction_db,
    };
    let settled = 72_000..96_000;
    let sibilant = |effects| mix_wave(sibilant_band, effects);
    let plain = sibilant(vec![]);
    let cut = sibilant(vec![cleanup(1, 100.0, deess(8.0))]);
    let band = db_between(&cut[settled.clone()], &plain[settled.clone()]);
    assert!(band <= -4.0, "sibilant band only fell {band} dB");
    let voiced = |effects| mix_wave(voiced_stack, effects);
    let plain = voiced(vec![]);
    let kept = voiced(vec![cleanup(1, 100.0, deess(8.0))]);
    let stack = db_between(&kept[settled.clone()], &plain[settled]);
    assert!(stack.abs() < 1.0, "voiced stack moved {stack} dB");
    assert_bypass_is_identity(sibilant_band, deess(8.0), deess(0.0));
    // Zero delay: the stage adds no latency to the chain.
    let h = Hooks::default();
    let p = with_cleanup(fixture_of(2), vec![cleanup(1, 100.0, deess(8.0))]);
    assert_eq!(
        AudioMixer::new(&p, 2, 0, &h)
            .unwrap()
            .cleanup_latency_samples(),
        0
    );
}

#[test]
fn dereverb_lowers_a_decaying_tail_keeps_the_direct_onset_and_bypass_is_identity() {
    // Stated limits at amount 100 % and RT60 0.8 s: the energy of a tail that decays by exactly
    // that RT60 falls by at least 4 dB between 0.5 s and 1.0 s, the first 10 ms (before the
    // 50 ms late delay applies) stay within 1 dB, and the stage's one-frame delay is paid by
    // the graph.
    let dereverb = |amount_percent: f32| project::CleanupKind::Dereverb {
        amount_percent,
        rt60_seconds: 0.8,
    };
    let plain = mix_wave(decaying_tail, vec![]);
    let tail = mix_wave(decaying_tail, vec![cleanup(1, 100.0, dereverb(100.0))]);
    let late = 24_000..48_000;
    let fall = db_between(&tail[late.clone()], &plain[late]);
    assert!(fall <= -4.0, "tail energy only fell {fall} dB");
    let onset = db_between(&tail[..480], &plain[..480]);
    assert!(onset.abs() < 1.0, "direct onset moved {onset} dB");
    assert_bypass_is_identity(decaying_tail, dereverb(100.0), dereverb(0.0));
    let h = Hooks::default();
    let p = with_cleanup(fixture_of(2), vec![cleanup(1, 100.0, dereverb(100.0))]);
    assert_eq!(
        AudioMixer::new(&p, 2, 0, &h)
            .unwrap()
            .cleanup_latency_samples(),
        2048
    );
    // Any block partition gives the same samples.
    let other = mix_synth(&p, &mut Wave(decaying_tail), 2, &[1, 7, 1031, 4093]);
    assert_eq!(tail, other);
}

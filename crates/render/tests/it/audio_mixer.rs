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
    let mut data = project::Project::empty(project::ProjectId(1), "".into())
        .unwrap()
        .into_data();
    let asset = testkit::source::asset(1, std::env::temp_dir().join("mixer.wav"))
        .format("wav")
        .duration(tick(1, 1))
        .content_hash(media::ContentHash::from_bytes(&[42]))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(tick(1, 1))
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
        items: vec![project::Item {
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            retime: Default::default(),
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
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

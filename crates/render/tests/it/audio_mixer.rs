#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use crate::common::*;
use project::{AnimationPath, Channel, Extrapolation, Keyframe, Param, ParamValue};
use render::*;
use time::{Tick, TimeRange};

struct Signal {
    level: f32,
    impulse: bool,
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
    let asset = media::SourceAsset::try_from(media::AssetData {
        ephemeral: false,
        id: media::AssetId(1),
        path: std::env::temp_dir().join("mixer.wav"),
        content_hash: media::ContentHash::from_bytes(&[42]),
        probe: media::SourceProbe {
            format: "wav".into(),
            duration: Some(tick(1, 1)),
            streams: vec![media::SourceStream {
                index: 0,
                kind: media::StreamKind::Audio,
                codec: "pcm_f32le".into(),
                time_base: media::TimeBase::new(1, 48000).unwrap(),
                start: None,
                duration: Some(tick(1, 1)),
                frame_rate: None,
                frame_count: None,
                variable_frame_rate: None,
                video: None,
                audio: Some(media::AudioInfo {
                    sample_rate: 48000,
                    channels: 2,
                    initial_padding: 0,
                    trailing_padding: 0,
                }),
            }],
        },
    })
    .unwrap();
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
            content: project::ItemParams::Audio(project::AudioParams {
                volume: 0.0,
                muted: false,
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

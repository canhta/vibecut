#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::common::{Empty, Hooks};
use project::*;
use render::*;
use render::{Error, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use time::{SourceRange, Tick};

fn scene(kind: &str) -> Project {
    let mut d = Project::empty(ProjectId(0), "transition goldens".into())
        .unwrap()
        .into_data();
    d.canvas.width = 4;
    d.canvas.height = 4;
    d.scene_mut().timeline.main.items = [(10, 0, [255, 0, 0, 255]), (11, 100, [0, 0, 255, 255])]
        .into_iter()
        .map(|(id, start, c)| Item {
            retime: Default::default(),
            id: ItemId(id),
            source_asset: None,
            source_range: SourceRange::new(Tick(100), Tick(200)).unwrap(),
            start: Tick(start),
            content: ItemParams::Overlay(OverlayParams { color: Color(c) }),
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
        })
        .collect();
    d.scene_mut().timeline.transitions.push(Transition {
        id: TransitionId(1),
        track: TrackId(0),
        from: ItemId(10),
        to: ItemId(11),
        kind: kind.into(),
        start: Tick(80),
        duration: Tick(40),
        alignment: TransitionAlignment::Center,
    });
    Project::new(d).unwrap()
}

#[test]
fn every_catalog_kind_has_start_mid_end_goldens_without_flash_or_frame_drop() {
    for def in TRANSITION_DEFINITIONS {
        let p = scene(def.kind);
        let h = Hooks::default();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        for (at, expected) in [
            (79, [1., 0., 0., 1.]),
            (80, [1., 0., 0., 1.]),
            (100, def.dip.unwrap_or([0.5, 0., 0.5, 1.])),
            (120, [0., 0., 1., 1.]),
            (121, [0., 0., 1., 1.]),
        ] {
            let im = r.render_frame(Tick(at), &mut Empty, &h).unwrap();
            assert!(
                im.pixels()
                    .iter()
                    .all(|p| p.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6)),
                "{} at {at}",
                def.kind
            );
        }
        // Every tick around and across the half-open span yields opaque pixels;
        // the final in-span frame converges to the exact incoming endpoint.
        let mut previous = None;
        for at in 79..=121 {
            let im = r.render_frame(Tick(at), &mut Empty, &h).unwrap();
            assert!(im.pixels().iter().all(|p| p[3] == 1.));
            if let Some(prev) = previous {
                assert!(
                    im.pixels()[0]
                        .iter()
                        .zip(prev)
                        .all(|(a, b): (&f32, f32)| (a - b).abs() <= 0.051)
                );
            }
            previous = Some(im.pixels()[0]);
        }
    }
}

#[test]
fn item_effects_masks_opacity_and_muted_hidden_tracks_compose_before_transition() {
    let p = scene("cross_dissolve");
    let mut d = p.into_data();
    for item in &mut d.scene_mut().timeline.main.items {
        item.effects.push(EffectInstance {
            id: "blur".into(),
            enabled: true,
            params: EffectParams::Blur { intensity: 10. },
        });
        item.masks
            .push(Mask::new("rectangle", MaskShape::Rectangle));
        item.animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(0.5)).unwrap(),
            )
            .unwrap();
    }
    d.scene_mut().timeline.main.muted = true;
    let p = Project::new(d).unwrap();
    let h = Hooks::default();
    let im = Renderer::new(&p, RenderSettings::from_project(&p))
        .unwrap()
        .render_frame(Tick(100), &mut Empty, &h)
        .unwrap();
    // Compare the midpoint with independent evaluations of each already-filtered
    // side over the same backdrop, through the ordinary item compositor.
    let mut sides = Vec::new();
    for id in [10, 11] {
        let mut d = p.clone().into_data();
        d.scene_mut().timeline.transitions.clear();
        d.scene_mut()
            .timeline
            .main
            .items
            .retain(|i| i.id == ItemId(id));
        let side = &mut d.scene_mut().timeline.main.items[0];
        side.start = Tick(0);
        let side = Project::new(d).unwrap();
        sides.push(
            Renderer::new(&side, RenderSettings::from_project(&side))
                .unwrap()
                .render_frame(Tick(50), &mut Empty, &h)
                .unwrap(),
        );
    }
    for ((actual, a), b) in im
        .pixels()
        .iter()
        .zip(sides[0].pixels())
        .zip(sides[1].pixels())
    {
        assert!(
            actual
                .iter()
                .zip(a.iter().zip(b))
                .all(|(d, (a, b))| (d - (a + b) * 0.5).abs() < 1e-6)
        );
    }
    let mut d = p.into_data();
    d.scene_mut().timeline.main.hidden = true;
    let hidden = Project::new(d).unwrap();
    let im = Renderer::new(&hidden, RenderSettings::from_project(&hidden))
        .unwrap()
        .render_frame(Tick(100), &mut Empty, &h)
        .unwrap();
    assert!(im.pixels().iter().all(|p| *p == [0., 0., 0., 1.]));
}

struct Cancel {
    checks: AtomicUsize,
    cancel_at: usize,
    inner: Hooks,
}
impl ExecutionHooks for Cancel {
    fn cancelled(&self) -> bool {
        self.checks.fetch_add(1, Ordering::SeqCst) >= self.cancel_at
    }
    fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
        self.inner.reserve(kind, bytes)
    }
}
#[test]
fn cancellation_between_sides_drops_all_leases_and_next_frame_succeeds() {
    let p = scene("cross_dissolve");
    let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let baseline = Cancel {
        checks: AtomicUsize::new(0),
        cancel_at: usize::MAX,
        inner: Hooks::default(),
    };
    drop(r.render_frame(Tick(100), &mut Empty, &baseline).unwrap());
    let checkpoints = baseline.checks.load(Ordering::SeqCst);
    for cancel_at in 0..checkpoints {
        let h = Cancel {
            checks: AtomicUsize::new(0),
            cancel_at,
            inner: Hooks::default(),
        };
        assert!(matches!(
            r.render_frame(Tick(100), &mut Empty, &h),
            Err(Error::Cancelled)
        ));
        assert_eq!(h.inner.used.load(Ordering::SeqCst), 0);
    }
    let h = Hooks::default();
    drop(r.render_frame(Tick(100), &mut Empty, &h).unwrap());
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
}

#[test]
fn transition_consumes_both_video_handles_at_exact_source_ticks() {
    struct Sources {
        calls: Vec<(ItemId, Tick)>,
    }
    impl SourceProvider for Sources {
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
            id: ItemId,
            at: Tick,
            h: &dyn ExecutionHooks,
        ) -> Result<Option<Image>> {
            self.calls.push((id, at));
            Ok(Some(Image::new(
                4,
                4,
                if id == ItemId(10) {
                    [1., 0., 0., 1.]
                } else {
                    [0., 0., 1., 1.]
                },
                h,
            )?))
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: ItemId,
            _: i64,
            n: usize,
            rate: u32,
            h: &dyn ExecutionHooks,
        ) -> Result<AudioBlock> {
            AudioBlock::new(rate, 2, n, MemoryKind::Decoded, h)
        }
    }
    let mut d = scene("cross_dissolve").into_data();
    d.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(1),
            path: std::env::temp_dir().join("transition-pixels"),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "synthetic".into(),
                duration: Some(Tick(1000)),
                streams: vec![media::SourceStream {
                    index: 0,
                    kind: media::StreamKind::Video,
                    codec: "rgba".into(),
                    time_base: media::TimeBase::new(1, 30).unwrap(),
                    start: None,
                    duration: Some(Tick(1000)),
                    frame_rate: Some(time::FrameRate::FPS_30),
                    frame_count: None,
                    variable_frame_rate: None,
                    video: Some(media::VideoInfo {
                        width: 4,
                        height: 4,
                        color: media::ColorInfo::default(),
                        bit_depth: Some(8),
                    }),
                    audio: None,
                }],
            },
        })
        .unwrap(),
    );
    for i in &mut d.scene_mut().timeline.main.items {
        i.source_asset = Some(media::AssetId(1));
        i.content = ItemParams::Video(VideoParams {
            source_audio_enabled: false,
            volume: 0.,
            muted: false,
            fades: Default::default(),
        });
    }
    let p = Project::new(d).unwrap();
    let h = Hooks::default();
    let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let mut sources = Sources { calls: vec![] };
    for (at, calls) in [
        (80, vec![(ItemId(10), Tick(180)), (ItemId(11), Tick(80))]),
        (100, vec![(ItemId(10), Tick(200)), (ItemId(11), Tick(100))]),
        (120, vec![(ItemId(11), Tick(120))]),
    ] {
        sources.calls.clear();
        drop(r.render_frame(Tick(at), &mut sources, &h).unwrap());
        assert_eq!(sources.calls, calls);
    }
    for hidden_side in [10, 11] {
        let mut d = p.clone().into_data();
        d.scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|i| i.id == ItemId(hidden_side))
            .unwrap()
            .animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(0.)).unwrap(),
            )
            .unwrap();
        let p = Project::new(d).unwrap();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        sources.calls.clear();
        let frame = r.render_frame(Tick(100), &mut sources, &h).unwrap();
        assert_eq!(sources.calls.len(), 1);
        assert_ne!(sources.calls[0].0, ItemId(hidden_side));
        let expected = if hidden_side == 10 {
            [0., 0., 0.5, 1.]
        } else {
            [0.5, 0., 0., 1.]
        };
        assert!(frame.pixels().iter().all(|p| *p == expected));
    }
}

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use crate::support::*;
use project::{
    Color, EffectParams, Item, ItemId, ItemParams, OverlayParams, Project, ProjectData, Timeline,
    Track, TrackId, TrackKind, effect_definition,
};
use render::*;
use time::{SourceRange, Tick, TimeRange};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn seconds(numerator: i64, denominator: i64) -> Tick {
    tick(numerator, denominator)
}

fn overlay(id: u64, color: [u8; 4], start: Tick, length: Tick) -> Item {
    testkit::item::item(
        ItemId(id),
        ItemParams::Overlay(OverlayParams {
            color: Color(color),
        }),
    )
    .source(SourceRange::new(Tick::ZERO, length).unwrap())
    .start(start)
    .build()
}

/// An invert adjustment layer over `[0, 1 s)`.
fn invert_layer(id: u64) -> Item {
    testkit::item::item(ItemId(id), ItemParams::Effect(EffectParams::default()))
        .source(SourceRange::new(Tick::ZERO, seconds(1, 1)).unwrap())
        .effects(vec![effect_definition("invert").unwrap().instance("fx")])
        .build()
}

fn track(id: u64, kind: TrackKind, items: Vec<Item>) -> Track {
    Track {
        id: TrackId(id),
        name: "".into(),
        kind,
        muted: false,
        hidden: false,
        locked: false,
        items,
    }
}

fn nested(main: Vec<Item>, overlays: Vec<Track>) -> Timeline {
    Timeline {
        transitions: Vec::new(),
        overlays,
        main: track(11, TrackKind::Video, main),
        magnet: false,
        audio: Vec::new(),
    }
}

fn canvas() -> ProjectData {
    let mut data = Project::empty(project::ProjectId(3), "".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 36;
    data
}

fn frame(project: &Project, at: Tick) -> Vec<[f32; 4]> {
    let hooks = Hooks::default();
    Renderer::new(project, RenderSettings::from_project(project))
        .unwrap()
        .render_frame(at, &mut Empty, &hooks)
        .unwrap()
        .pixels()
        .to_vec()
}

fn centre(pixels: &[[f32; 4]]) -> [f32; 4] {
    pixels[18 * 64 + 32]
}

fn difference(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter()
        .zip(b)
        .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
        .fold(0.0, f32::max)
}

#[test]
fn an_adjustment_layer_inside_a_compound_clip_leaves_the_track_below_alone() {
    let blue = |layer: Option<Track>| {
        let mut data = canvas();
        let timeline = &mut data.scene_mut().timeline;
        timeline.main.items = vec![overlay(1, BLUE, Tick::ZERO, seconds(1, 1))];
        if let Some(layer) = layer {
            timeline.insert_track(layer);
        }
        Project::new(data).unwrap()
    };
    let plain = frame(&blue(None), seconds(1, 2));
    // On the scene the layer inverts the blue below it.
    let outside = frame(
        &blue(Some(track(2, TrackKind::Effect, vec![invert_layer(3)]))),
        seconds(1, 2),
    );
    assert!(
        difference(&outside, &plain) > 0.5,
        "the layer inverts the scene"
    );
    // Inside a compound clip it adjusts only the compound clip's own transparent frame.
    let inner = nested(
        Vec::new(),
        vec![track(12, TrackKind::Effect, vec![invert_layer(13)])],
    );
    let compound = Item::compound(ItemId(10), Tick::ZERO, inner).unwrap();
    let inside = frame(
        &blue(Some(track(2, TrackKind::Video, vec![compound]))),
        seconds(1, 2),
    );
    assert!(difference(&inside, &plain) < 1e-4, "nothing leaks below");
}

/// Red over 0-1 s, green over 1-2 s and blue over 2-3 s of nested time.
fn stripes() -> Timeline {
    nested(
        vec![
            overlay(20, RED, Tick::ZERO, seconds(1, 1)),
            overlay(21, GREEN, seconds(1, 1), seconds(1, 1)),
            overlay(22, BLUE, seconds(2, 1), seconds(1, 1)),
        ],
        Vec::new(),
    )
}

fn scene(items: Vec<Item>) -> Project {
    let mut data = canvas();
    data.scene_mut().timeline.main.items = items;
    Project::new(data).unwrap()
}

fn is(pixel: [f32; 4], color: [u8; 4]) -> bool {
    pixel
        .iter()
        .zip(color)
        .take(3)
        .all(|(value, byte)| (value - f32::from(byte) / 255.0).abs() < 0.02)
}

#[test]
fn a_trimmed_double_speed_compound_clip_shows_the_nested_frame_at_its_source_time() {
    let mut compound = Item::compound(ItemId(10), Tick::ZERO, stripes()).unwrap();
    compound.source_range = SourceRange::new(seconds(1, 1), seconds(3, 1)).unwrap();
    compound.retime.speed = time::Speed::from_f64(2.0).unwrap();
    let project = scene(vec![compound]);
    assert_eq!(project.duration(), seconds(1, 1));
    // Output 0.25 s reads nested 1.5 s (green); output 0.75 s reads nested 2.5 s (blue).
    assert!(is(centre(&frame(&project, seconds(1, 4))), GREEN));
    assert!(is(centre(&frame(&project, seconds(3, 4))), BLUE));
}

#[test]
fn the_halves_of_a_split_compound_clip_render_their_own_windows() {
    let mut left = Item::compound(ItemId(10), Tick::ZERO, stripes()).unwrap();
    left.source_range = SourceRange::new(Tick::ZERO, seconds(3, 2)).unwrap();
    let mut right = Item::compound(ItemId(40), seconds(3, 2), stripes()).unwrap();
    right.source_range = SourceRange::new(seconds(3, 2), seconds(3, 1)).unwrap();
    let ItemParams::Compound(params) = &mut right.content else {
        panic!("compound clip");
    };
    let mut next = 100_u64;
    params
        .renumber(&mut || -> project::Result<u64> {
            next += 1;
            Ok(next)
        })
        .unwrap();
    let project = scene(vec![left, right]);
    assert!(is(centre(&frame(&project, seconds(1, 2))), RED));
    assert!(is(centre(&frame(&project, seconds(5, 4))), GREEN));
    assert!(is(centre(&frame(&project, seconds(7, 4))), GREEN));
    assert!(is(centre(&frame(&project, seconds(5, 2))), BLUE));
}

#[test]
fn an_effect_on_a_compound_clip_applies_to_its_composite() {
    let invert = || vec![effect_definition("invert").unwrap().instance("fx")];
    let mut red = overlay(1, RED, Tick::ZERO, seconds(1, 1));
    red.effects = invert();
    let direct = frame(&scene(vec![red]), seconds(1, 2));
    let mut compound = Item::compound(
        ItemId(10),
        Tick::ZERO,
        nested(
            vec![overlay(20, RED, Tick::ZERO, seconds(1, 1))],
            Vec::new(),
        ),
    )
    .unwrap();
    let plain = frame(&scene(vec![compound.clone()]), seconds(1, 2));
    compound.effects = invert();
    let inverted = frame(&scene(vec![compound]), seconds(1, 2));
    assert!(is(centre(&plain), RED));
    assert!(
        difference(&inverted, &plain) > 0.5,
        "the effect changes the composite"
    );
    assert!(
        difference(&inverted, &direct) < 1e-3,
        "as it changes the red clip itself"
    );
}

#[test]
fn a_preview_click_selects_the_compound_clip() {
    let compound = Item::compound(ItemId(10), Tick::ZERO, stripes()).unwrap();
    let project = scene(vec![compound]);
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let viewport = render::viewport::Viewport::new(
        Rect {
            x: 0.,
            y: 0.,
            width: 64.,
            height: 36.,
        },
        [64, 36],
        1.,
        [0., 0.],
    )
    .unwrap();
    assert_eq!(
        renderer
            .hit_test(&viewport, [32., 18.], seconds(1, 2))
            .unwrap(),
        Some(ItemId(10))
    );
}

struct Level(f32);
impl SourceProvider for Level {
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
        _: ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        Ok(None)
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: ItemId,
        _: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        let mut out = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
        for plane in out.planes_mut() {
            plane.iter_mut().for_each(|sample| *sample = self.0);
        }
        Ok(out)
    }
}

/// One second of audio item 1 after a half-second gap: on its own track, or inside compound
/// clip 10 whose own volume is `volume` dB.
fn audible(compound: Option<f32>) -> Project {
    let mut data = canvas();
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("compound.wav"))
            .format("wav")
            .duration(seconds(1, 1))
            .content_hash(media::ContentHash::from_bytes(&[7]))
            .stream(
                testkit::source::audio()
                    .codec("pcm_f32le")
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(seconds(1, 1))
                    .build(),
            )
            .build(),
    );
    let item = |start: Tick| {
        testkit::item::audio(ItemId(1))
            .asset(media::AssetId(1))
            .source(TimeRange::new(Tick::ZERO, seconds(1, 1)).unwrap())
            .start(start)
            .build()
    };
    let timeline = &mut data.scene_mut().timeline;
    match compound {
        Some(volume) => {
            let inner = Timeline {
                audio: vec![track(12, TrackKind::Audio, vec![item(Tick::ZERO)])],
                ..nested(Vec::new(), Vec::new())
            };
            let mut clip = Item::compound(ItemId(10), seconds(1, 2), inner).unwrap();
            if let ItemParams::Compound(params) = &mut clip.content {
                params.clip.volume = volume;
            }
            timeline.main.items = vec![clip];
        }
        None => timeline.insert_track(track(2, TrackKind::Audio, vec![item(seconds(1, 2))])),
    }
    Project::new(data).unwrap()
}

#[test]
fn a_compound_clip_sounds_as_its_nested_mix_through_its_own_volume() {
    let hooks = Hooks::default();
    // Two seconds, mixed as two one-second blocks (a block holds at most 65,536 frames).
    let mix = |project: &Project| {
        let mut planes = vec![Vec::new(); 2];
        for start in [0, 48_000] {
            let block = mix_audio(project, start, 48_000, 2, &mut Level(0.25), &hooks).unwrap();
            for (plane, part) in planes.iter_mut().zip(block.planes()) {
                plane.extend_from_slice(part);
            }
        }
        planes
    };
    let members = mix(&audible(None));
    assert_eq!(mix(&audible(Some(0.0))), members);
    assert!(members[0][48_000..].iter().any(|sample| *sample != 0.0));
    let quieter = mix(&audible(Some(-6.0)));
    let gain = 10f32.powf(-6.0 / 20.0);
    for (quiet, loud) in quieter[0].iter().zip(&members[0]).skip(24_000).take(48_000) {
        assert!((quiet - loud * gain).abs() < 1e-5);
    }
}

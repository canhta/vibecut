#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Stabilization: the warp a track derives, its zoom, and the picture render draws from it.
use crate::support::{Hooks, tick};
use project::StabilizeLevel;
use render::stabilize::{Pose, Track};
use render::{
    AudioBlock, ExecutionHooks, Image, MemoryKind, RenderSettings, Renderer, Result,
    SourceProvider, UnavailableReason,
};
use std::sync::Arc;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

const GRID: [u32; 2] = [480, 270];
const FPS: i64 = 30;

/// A deterministic handheld path: two sines per axis and a slow roll, as the clean-up study's fixture scaled to
/// the 480x270 analysis grid (14 px at 1080p is 3.5 px here).
fn shake(seconds: f64) -> Pose {
    let tau = std::f64::consts::TAU;
    Pose {
        tx: 3.5 * (tau * 0.9 * seconds).sin() + 1.5 * (tau * 4.3 * seconds).sin(),
        ty: 2.25 * (tau * 1.1 * seconds + 1.0).sin() + (tau * 5.1 * seconds).sin(),
        angle: 0.012 * (tau * 1.3 * seconds).sin() + 0.004 * (tau * 3.1 * seconds).sin(),
        ln_scale: 0.0,
    }
}
fn shaky_track(frames: usize) -> Track {
    let times: Vec<Tick> = (0..frames as i64).map(|n| tick(n, FPS)).collect();
    let poses: Vec<Pose> = (0..frames).map(|n| shake(n as f64 / FPS as f64)).collect();
    Track::new(GRID, times, &poses).unwrap()
}
fn whole(frames: usize) -> SourceRange {
    SourceRange::new(Tick::ZERO, tick(frames as i64, FPS)).unwrap()
}
/// Where the source frame's own point `p` (centred grid pixels) lands in the steadied output of
/// the frame at `at`: the inverse of the warp's linear sampling map.
fn lands(stabilizer: &render::stabilize::Stabilizer, at: Tick, p: [f64; 2]) -> [f64; 2] {
    let warp = stabilizer.warp(at).unwrap();
    let t = warp.source_of([0.0, 0.0], 1.0);
    let c1 = warp.source_of([1.0, 0.0], 1.0);
    let c2 = warp.source_of([0.0, 1.0], 1.0);
    let (a, b) = (c1[0] - t[0], c2[0] - t[0]);
    let (c, d) = (c1[1] - t[1], c2[1] - t[1]);
    let det = a * d - b * c;
    let (x, y) = (p[0] - t[0], p[1] - t[1]);
    [(d * x - b * y) / det, (-c * x + a * y) / det]
}
/// Frame-to-frame displacement of the four frame corners, RMS, as the clean-up study measures residual motion.
fn rms_motion(positions: &[[[f64; 2]; 4]]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for pair in positions.windows(2) {
        for (after, before) in pair[1].iter().zip(&pair[0]) {
            for (a, b) in after.iter().zip(before) {
                let delta = a - b;
                sum += delta * delta;
                count += 1.0;
            }
        }
    }
    (sum / count).sqrt()
}
fn corners() -> [[f64; 2]; 4] {
    let (hx, hy) = (f64::from(GRID[0]) / 2.0, f64::from(GRID[1]) / 2.0);
    [[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]]
}
/// The four frame corners of each frame, in order.
type CornerPath = Vec<[[f64; 2]; 4]>;
/// The corner paths of a clip in the raw footage and after the stabilizer at `level`.
fn corner_paths(track: &Track, level: StabilizeLevel, frames: usize) -> (CornerPath, CornerPath) {
    let stabilizer = track.stabilizer(level, whole(frames));
    let mut raw = Vec::new();
    let mut steady = Vec::new();
    for n in 0..frames {
        let at = tick(n as i64, FPS);
        let pose = track.pose(n).unwrap();
        let (sin, cos) = pose.angle.sin_cos();
        // The first frame's corner as the shaky camera sees it in frame n.
        let seen = corners().map(|c| {
            [
                c[0] * cos - c[1] * sin + pose.tx,
                c[0] * sin + c[1] * cos + pose.ty,
            ]
        });
        raw.push(seen);
        steady.push(seen.map(|p| lands(&stabilizer, at, p)));
    }
    (raw, steady)
}

#[test]
fn a_level_steadies_the_corner_path_and_a_steadier_level_steadies_more() {
    let frames = 300;
    let track = shaky_track(frames);
    let mut previous = f64::INFINITY;
    let (raw, _) = corner_paths(&track, StabilizeLevel::Normal, frames);
    let input = rms_motion(&raw);
    for level in StabilizeLevel::ALL {
        let (_, steady) = corner_paths(&track, level, frames);
        let output = rms_motion(&steady);
        // Zoom scales the picture, so compare against the input at 1080p scale as the study does:
        // the bound there is 1.0 px at 1080p against 4.3 px of input shake.
        assert!(output < input, "{level:?}: {output} !< {input}");
        assert!(
            output <= previous,
            "{level:?} must not move more than a lighter level"
        );
        previous = output;
    }
    // Normal and Stable meet the study's bound at 1080p scale (4x the analysis grid) of 1.0 px.
    for level in [StabilizeLevel::Normal, StabilizeLevel::Stable] {
        let (_, steady) = corner_paths(&track, level, frames);
        assert!(
            4.0 * rms_motion(&steady) <= 1.0,
            "{level:?}: {} px at 1080p",
            4.0 * rms_motion(&steady)
        );
    }
}

#[test]
fn no_level_shows_a_border_and_a_trim_never_needs_more_zoom() {
    let frames = 300;
    let track = shaky_track(frames);
    let half = [f64::from(GRID[0]) / 2.0, f64::from(GRID[1]) / 2.0];
    for level in StabilizeLevel::ALL {
        let stabilizer = track.stabilizer(level, whole(frames));
        assert!(stabilizer.zoom() >= 1.0 && stabilizer.zoom() < render::stabilize::MAX_ZOOM);
        for n in 0..frames {
            for corner in stabilizer.corner_sources(tick(n as i64, FPS)).unwrap() {
                assert!(
                    corner[0].abs() <= half[0] && corner[1].abs() <= half[1],
                    "{level:?} frame {n}: {corner:?} leaves the frame"
                );
            }
        }
        let trimmed = track.stabilizer(
            level,
            SourceRange::new(tick(60, FPS), tick(120, FPS)).unwrap(),
        );
        assert_eq!(trimmed.len(), 60);
        assert!(trimmed.zoom() <= stabilizer.zoom() + 1e-12);
        // Smoothing reads the whole track, so a trim keeps each kept frame's shift exactly.
        for n in 60..120 {
            let at = tick(n, FPS);
            assert_eq!(
                trimmed.warp(at).unwrap().source_of([0.0, 0.0], 1.0),
                stabilizer.warp(at).unwrap().source_of([0.0, 0.0], 1.0)
            );
        }
    }
    // A stronger level never zooms less on the same footage.
    let zoom = |level| track.stabilizer(level, whole(frames)).zoom();
    assert!(zoom(StabilizeLevel::Flexible) <= zoom(StabilizeLevel::Stable) + 1e-9);
}

#[test]
fn a_track_encodes_to_the_same_bytes_and_reads_back_whole() {
    let track = shaky_track(90);
    let bytes = track.encode();
    assert_eq!(bytes, shaky_track(90).encode());
    assert_eq!(Track::decode(&bytes).unwrap(), track);
    assert!(Track::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(Track::decode(&bad).is_err());
    // Frame times must strictly increase.
    let poses = [Pose::IDENTITY, Pose::IDENTITY];
    assert!(Track::new(GRID, vec![tick(1, FPS), tick(1, FPS)], &poses).is_err());
    assert!(Track::new(GRID, vec![], &[]).is_err());
}

/// A 32x32 source whose picture is a smooth diagonal ramp, so a warp changes every pixel.
struct Ramp;
impl SourceProvider for Ramp {
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
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        let bytes: Vec<u8> = (0..32 * 32)
            .flat_map(|i| {
                let (x, y) = (i % 32, i / 32);
                let v = (40 + 5 * x + 3 * y) as u8;
                [v, 255 - v, v / 2, 255]
            })
            .collect();
        Ok(Some(Image::from_rgba8(32, 32, &bytes, false, 13, hooks)?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, hooks)
    }
}
fn scene(stabilize: Option<StabilizeLevel>) -> project::Project {
    let asset = testkit::source::asset(1, std::env::temp_dir().join("render-stabilize"))
        .format("synthetic")
        .duration(tick(2, 1))
        .stream(
            testkit::source::video()
                .codec("rgba")
                .time_base(media::TimeBase::new(1, 30).unwrap())
                .duration(tick(2, 1))
                .start(Tick::ZERO)
                .frame_count(60)
                .variable_frame_rate(false)
                .size(32, 32)
                .build(),
        )
        .build();
    let mut data = project::Project::empty(project::ProjectId(7), "stabilize".into())
        .unwrap()
        .into_data();
    data.canvas.width = 32;
    data.canvas.height = 32;
    data.assets.push(asset);
    let mut item = testkit::item::video(project::ItemId(1))
        .asset(media::AssetId(1))
        .source(SourceRange::new(Tick::ZERO, tick(2, 1)).unwrap())
        .build();
    let project::ItemParams::Video(params) = &mut item.content else {
        panic!("video item");
    };
    params.stabilize = stabilize;
    data.scene_mut().timeline.main.items.push(item);
    project::Project::new(data).unwrap()
}
/// Poses 4 grid pixels apart one second in: enough camera motion that the warp is not the identity.
fn pair_track() -> Arc<Track> {
    let times = vec![Tick::ZERO, tick(1, 1)];
    let poses = [
        Pose::IDENTITY,
        Pose {
            tx: 4.0,
            ty: -2.0,
            angle: 0.02,
            ln_scale: 0.0,
        },
    ];
    Arc::new(Track::new([32, 32], times, &poses).unwrap())
}

#[test]
fn a_stabilized_item_renders_the_same_in_preview_and_export_and_a_missing_track_renders_uncorrected()
 {
    let hooks = Hooks::default();
    let level = Some(StabilizeLevel::Normal);
    let render = |project: &project::Project, track: Option<Arc<Track>>, preview: bool| {
        let mut settings = RenderSettings::from_project(project);
        if let Some(track) = track {
            settings.stabilization.insert(project::ItemId(1), track);
        }
        let mut renderer = Renderer::new(project, settings).unwrap();
        if preview {
            renderer
                .preview_frame(Tick::ZERO, &mut Ramp, &hooks)
                .unwrap()
        } else {
            renderer
                .render_frame(Tick::ZERO, &mut Ramp, &hooks)
                .unwrap()
        }
    };
    let off = scene(None);
    let on = scene(level);
    let plain = render(&off, None, false);
    // Switched on with no analysis: never an error, and the picture is the uncorrected one.
    assert_eq!(render(&on, None, false).pixels(), plain.pixels());
    // A track for an item whose switch is off changes nothing.
    assert_eq!(
        render(&off, Some(pair_track()), false).pixels(),
        plain.pixels()
    );
    // Switched on with a track: the picture is warped, and preview equals export.
    let export = render(&on, Some(pair_track()), false);
    let preview = render(&on, Some(pair_track()), true);
    assert_eq!(export.pixels(), preview.pixels());
    assert_ne!(export.pixels(), plain.pixels());
    // The zoom keeps the picture inside the frame: no transparent border pixel.
    assert!(export.pixels().iter().all(|pixel| pixel[3] > 0.999));
    // Two renders from the same inputs are byte-equal.
    assert_eq!(
        render(&on, Some(pair_track()), false).pixels(),
        export.pixels()
    );
}

#[test]
fn a_clip_longer_than_its_track_still_has_a_warp_at_every_frame() {
    let track = shaky_track(30);
    let stabilizer = track.stabilizer(
        StabilizeLevel::Normal,
        SourceRange::new(Tick::ZERO, Tick(10 * TICKS_PER_SECOND)).unwrap(),
    );
    assert_eq!(stabilizer.len(), 30);
    assert!(stabilizer.warp(Tick(10 * TICKS_PER_SECOND)).is_some());
}

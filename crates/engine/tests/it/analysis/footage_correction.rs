#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and assertions"
)]
//! Footage correction: stabilize and the person matte measured once per source, looked up by
//! key, applied at render.
use engine::{
    ByteBudget, Command, RuntimeError, Session,
    cache::ArtifactCache,
    footage::{Correction, CorrectionJob, CorrectionKind, CorrectionRequest, Segmenter, resolve},
    jobs::{JobRunner, JobState, RunnerConfig, SystemClock},
};
use media::{AssetId, SourceAsset};
use project::{ItemId, Project, ProjectId, StabilizeLevel};
use render::stabilize::{Stabilizer, Track};
use std::{
    io::Write,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{TICKS_PER_SECOND, Tick};

const FPS: i64 = 30;

/// Hash of a lattice point in `0..1`.
fn hash(x: i64, y: i64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h % 10_000) as f64 / 10_000.0
}
/// Smooth value noise with lattice spacing `cell`.
fn lattice(x: f64, y: f64, cell: f64) -> f64 {
    let (gx, gy) = (x / cell, y / cell);
    let (ix, iy) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - ix, gy - iy);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (ix as i64, iy as i64);
    let top = hash(ix, iy) * (1.0 - sx) + hash(ix + 1, iy) * sx;
    let bottom = hash(ix, iy + 1) * (1.0 - sx) + hash(ix + 1, iy + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}
/// The world: texture at every scale a block matcher needs.
fn texture(x: f64, y: f64) -> f64 {
    0.55 * lattice(x, y, 11.0) + 0.3 * lattice(x, y, 5.0) + 0.15 * lattice(x, y, 23.0)
}
/// Camera pose of frame `n`: `[tx, ty, angle]` in grid pixels and radians. The clean-up study's handheld fixture
/// at the 480x270 grid: 14 px at 1080p is 3.5 px here.
fn camera(n: usize) -> [f64; 3] {
    let t = n as f64 / FPS as f64;
    let tau = std::f64::consts::TAU;
    [
        3.5 * (tau * 0.9 * t).sin() + 1.5 * (tau * 4.3 * t).sin(),
        2.25 * (tau * 1.1 * t + 1.0).sin() + (tau * 5.1 * t).sin(),
        0.012 * (tau * 1.3 * t).sin() + 0.004 * (tau * 3.1 * t).sin(),
    ]
}
fn map(pose: [f64; 3], p: [f64; 2]) -> [f64; 2] {
    let (s, c) = pose[2].sin_cos();
    [c * p[0] - s * p[1] + pose[0], s * p[0] + c * p[1] + pose[1]]
}
fn unmap(pose: [f64; 3], q: [f64; 2]) -> [f64; 2] {
    let (s, c) = pose[2].sin_cos();
    let (x, y) = (q[0] - pose[0], q[1] - pose[1]);
    [c * x + s * y, -s * x + c * y]
}
/// Fixture ownership is per process, like the scene detection fixtures.
fn fixture(name: &str, width: u32, height: u32, frames: usize) -> SourceAsset {
    let dir = testkit::fixture::dir("footage-correction");
    let path = testkit::fixture::publish(&dir, &format!("{name}.mkv"), |path| {
        let mut child = testkit::oracle::command("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgb24",
                "-video_size",
            ])
            .arg(format!("{width}x{height}"))
            .args([
                "-framerate",
                "30",
                "-i",
                "pipe:0",
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "bgr0",
            ])
            .arg(path)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let half = [f64::from(width) / 2.0, f64::from(height) / 2.0];
        for n in 0..frames {
            let mut pixels = Vec::with_capacity((width * height * 3) as usize);
            for y in 0..height {
                for x in 0..width {
                    let centred = [f64::from(x) + 0.5 - half[0], f64::from(y) + 0.5 - half[1]];
                    let world = unmap(camera(n), centred);
                    let grey = (texture(world[0] + half[0], world[1] + half[1]) * 255.0) as u8;
                    pixels.extend_from_slice(&[grey; 3]);
                }
            }
            input.write_all(&pixels).unwrap();
        }
        drop(input);
        assert!(child.wait().unwrap().success());
    });
    SourceAsset::import(AssetId(1), path).unwrap()
}

fn setup() -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    let budget = ByteBudget::new(512 << 20).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 16).unwrap();
    let runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    (runner, cache, dir)
}
fn wait(runner: &mut JobRunner, job: &CorrectionJob) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(job.id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return state;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
/// One analysis in its own runner and cache: the track and the exact bytes the job retained.
fn analyze(asset: &SourceAsset) -> (Arc<Track>, Vec<u8>) {
    let (mut runner, mut cache, _dir) = setup();
    let request = CorrectionRequest::new(asset.clone(), CorrectionKind::Stabilize).unwrap();
    assert_eq!(request.cached(&mut cache).unwrap(), None);
    let job = request.submit(&mut runner, &mut cache, None).unwrap();
    assert_eq!(
        wait(&mut runner, &job),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|s| s.error)
    );
    let stored = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    // A second look finds the published entry and the same track.
    assert_eq!(request.cached(&mut cache).unwrap(), Some(stored.clone()));
    let Correction::Stabilize(track) = stored else {
        panic!("stabilize track");
    };
    (track, runner.output_bytes(job.id).unwrap().to_vec())
}
fn whole(track: &Track) -> time::SourceRange {
    time::SourceRange::new(
        Tick::ZERO,
        track
            .times()
            .last()
            .unwrap()
            .saturating_add(Tick(TICKS_PER_SECOND / FPS)),
    )
    .unwrap()
}
/// Where a frame-0 point lands in the steadied output of the frame at `at`.
fn lands(stabilizer: &Stabilizer, at: Tick, p: [f64; 2]) -> [f64; 2] {
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
fn rms_motion(paths: &[[[f64; 2]; 4]]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for pair in paths.windows(2) {
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

#[test]
fn stabilize_leaves_less_residual_motion_than_the_bound_and_never_shows_a_border() {
    let (width, height, frames) = (480u32, 270u32, 90usize);
    let track = analyze(&fixture("shaky-480-90", width, height, frames)).0;
    assert_eq!(track.len(), frames);
    assert_eq!(track.grid(), [width, height]);
    let half = [f64::from(width) / 2.0, f64::from(height) / 2.0];
    let corners = [
        [-half[0], -half[1]],
        [half[0], -half[1]],
        [half[0], half[1]],
        [-half[0], half[1]],
    ];
    // Where frame 0's corners really are in frame n: the fixture's own camera path.
    let first = camera(0);
    let seen: Vec<[[f64; 2]; 4]> = (0..frames)
        .map(|n| corners.map(|c| map(camera(n), unmap(first, c))))
        .collect();
    // The study measured the input at 4.3 px of corner motion at 1080p and bounds the output at 1.0.
    let input = 4.0 * rms_motion(&seen);
    assert!(input > 2.0, "fixture must shake: {input} px at 1080p");
    for level in StabilizeLevel::ALL {
        let stabilizer = track.stabilizer(level, whole(&track));
        let steady: Vec<[[f64; 2]; 4]> = track
            .times()
            .iter()
            .zip(&seen)
            .map(|(at, corners)| corners.map(|p| lands(&stabilizer, *at, p)))
            .collect();
        let output = 4.0 * rms_motion(&steady);
        assert!(output < input, "{level:?}: {output} !< {input}");
        if level != StabilizeLevel::Flexible {
            assert!(output <= 1.0, "{level:?}: {output} px at 1080p exceeds 1.0");
        }
        // No border at any level: every output corner reads inside the frame.
        for at in track.times() {
            for corner in stabilizer.corner_sources(*at).unwrap() {
                assert!(
                    corner[0].abs() <= half[0] && corner[1].abs() <= half[1],
                    "{level:?} at {at:?}: {corner:?}"
                );
            }
        }
    }
}

#[test]
fn two_uncached_runs_retain_the_same_bytes() {
    let asset = fixture("shaky-240", 240, 135, 12);
    let (first, first_bytes) = analyze(&asset);
    let (second, second_bytes) = analyze(&asset);
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first.encode(), second.encode());
    assert_eq!(first_bytes, first.encode());
    // The fixture moves, so equal bytes are not just equal identities.
    assert!((1..first.len()).any(|n| first.pose(n).unwrap().tx != 0.0));
}

fn session_over(asset: SourceAsset, frames: i64) -> (Session, ItemId) {
    let mut data = Project::empty(ProjectId(1), "Footage".into())
        .unwrap()
        .into_data();
    data.assets.push(asset);
    let mut item = testkit::item::video(ItemId(10))
        .asset(AssetId(1))
        .source(time::SourceRange::new(Tick::ZERO, Tick(frames * TICKS_PER_SECOND / FPS)).unwrap())
        .build();
    let project::ItemParams::Video(params) = &mut item.content else {
        panic!("video item");
    };
    params.stabilize = Some(StabilizeLevel::Normal);
    data.scene_mut().timeline.main.items.push(item);
    (
        Session::new(Project::new(data).unwrap(), 30).unwrap(),
        ItemId(10),
    )
}

#[test]
fn a_cleared_cache_needs_analysis_and_export_refuses_but_split_keeps_the_correction() {
    let asset = fixture("shaky-240", 240, 135, 12);
    let (mut s, item) = session_over(asset.clone(), 12);
    let (mut runner, mut cache, _dir) = setup();
    // Nothing analysed yet: the item is listed as needing analysis and nothing fails.
    let before = resolve(s.project(), &mut cache);
    assert_eq!(before.missing, vec![(item, CorrectionKind::Stabilize)]);
    assert!(before.tracks.is_empty());
    let request = CorrectionRequest::new(asset, CorrectionKind::Stabilize).unwrap();
    let job = request.submit(&mut runner, &mut cache, None).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    job.finish(&mut runner, &mut cache).unwrap().unwrap();
    let ready = resolve(s.project(), &mut cache);
    assert!(ready.missing.is_empty());
    assert!(ready.tracks.contains_key(&item));
    // A split needs no new analysis: both halves find the same stored track.
    s.apply(Command::SplitAt {
        item,
        at: Tick(TICKS_PER_SECOND / 10),
    })
    .unwrap();
    let ids: Vec<ItemId> = s
        .project()
        .data()
        .scene()
        .timeline
        .main
        .items
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ids.len(), 2);
    let split = resolve(s.project(), &mut cache);
    assert!(split.missing.is_empty(), "{:?}", split.missing);
    assert!(Arc::ptr_eq(&split.tracks[&ids[0]], &split.tracks[&ids[1]]));
    // A level change keeps the key too: only the item parameter differs.
    let level = Command::SetItemField {
        item: ids[0],
        patch: engine::ItemParamPatch::Stabilize(Some(StabilizeLevel::Stable)),
    };
    s.apply(level).unwrap();
    assert!(resolve(s.project(), &mut cache).missing.is_empty());
    // A cleared cache is Needs analysis again, and export refuses naming both items.
    let (_runner, mut empty, _other) = setup();
    let cleared = resolve(s.project(), &mut empty);
    assert_eq!(
        cleared.missing,
        ids.iter()
            .map(|id| (*id, CorrectionKind::Stabilize))
            .collect::<Vec<_>>()
    );
    let project = s.project().clone();
    let mut export = render::ExportSettings::from_project(&project);
    cleared.attach(&mut export.render);
    assert_export_refused(&project, export, &ids);
}

/// Export of `project` with `export` refuses before starting, naming exactly `ids`.
fn assert_export_refused(project: &Arc<Project>, export: render::ExportSettings, ids: &[ItemId]) {
    let key = engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::Preview,
        source: [0; 32],
        provider: "test",
        model: "export",
        version: "1",
        settings: b"footage",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    let (mut export_runner, _cache, _dir) = setup();
    let refused = engine::submit_export(
        &mut export_runner,
        key,
        project.clone(),
        time::TimeRange::new(Tick::ZERO, project.duration()).unwrap(),
        export,
        std::env::temp_dir().join("vibecut-footage-refused.mp4"),
    );
    match refused {
        Err(RuntimeError::CorrectionMissing(items)) => assert_eq!(items, ids),
        Err(other) => panic!("wrong refusal: {other:?}"),
        Ok(_) => panic!("export started without its correction"),
    }
}

/// A stand-in segmenter: a deterministic plane from the frame's own pixels (bright is a person),
/// counting its calls. The model's accuracy is the segmentation layer's; this proves the job.
fn brightness_segmenter(calls: Arc<std::sync::atomic::AtomicUsize>) -> Segmenter {
    Box::new(
        move |frame: ai::vision::PersonFrame<'_>, hooks: &dyn ai::ExecutionHooks| {
            if hooks.cancelled() {
                return Err(ai::Error::Cancelled);
            }
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let cells = ai::vision::MATTE_SIDE * ai::vision::MATTE_SIDE;
            let values = frame.values();
            ai::vision::PersonMatte::new(
                (0..cells)
                    .map(|i| {
                        let mean = (values[i] + values[cells + i] + values[2 * cells + i]) / 765.0;
                        if mean > 0.5 { 1.0 } else { mean * 0.5 }
                    })
                    .collect(),
            )
        },
    )
}
/// One matte analysis in its own runner and cache: the matte and the exact bytes retained.
fn analyze_matte(
    asset: &SourceAsset,
    calls: &Arc<std::sync::atomic::AtomicUsize>,
) -> (Arc<render::matte::MatteTrack>, Vec<u8>) {
    let (mut runner, mut cache, _dir) = setup();
    let request = CorrectionRequest::new(asset.clone(), CorrectionKind::PersonMatte).unwrap();
    assert_eq!(request.cached(&mut cache).unwrap(), None);
    // A cold matte without a segmenter is the typed missing-model refusal.
    match request.submit(&mut runner, &mut cache, None) {
        Err(engine::AnalysisError::Providers(engine::providers::Error::ModelMissing(id))) => {
            assert_eq!(id, engine::footage::PERSON_MODEL)
        }
        Err(other) => panic!("wrong refusal: {other:?}"),
        Ok(_) => panic!("a cold matte started without its model"),
    }
    let job = request
        .submit(
            &mut runner,
            &mut cache,
            Some(brightness_segmenter(calls.clone())),
        )
        .unwrap();
    assert_eq!(
        wait(&mut runner, &job),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|s| s.error)
    );
    let stored = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(request.cached(&mut cache).unwrap(), Some(stored.clone()));
    // A warm entry completes without the model or the source.
    let warm = request.submit(&mut runner, &mut cache, None).unwrap();
    assert_eq!(wait(&mut runner, &warm), JobState::Completed);
    let Correction::PersonMatte(matte) = stored else {
        panic!("person matte");
    };
    (matte, runner.output_bytes(job.id).unwrap().to_vec())
}

#[test]
fn a_person_matte_round_trips_one_plane_per_frame_and_two_runs_retain_the_same_bytes() {
    let frames = 12;
    let asset = fixture("shaky-240", 240, 135, frames);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (first, first_bytes) = analyze_matte(&asset, &calls);
    assert_eq!(first.len(), frames);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), frames);
    let (second, second_bytes) = analyze_matte(&asset, &calls);
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first, second);
    assert_eq!(first_bytes, first.encode());
    assert_eq!(
        render::matte::MatteTrack::decode(&first_bytes).unwrap(),
        *first
    );
    // The stand-in sees the texture, so the stored planes are not flat.
    let plane = first.plane(0).unwrap();
    assert!(plane.contains(&255) && plane.iter().any(|v| *v < 128));
    // The two kinds never share a key.
    assert_ne!(
        CorrectionRequest::new(asset.clone(), CorrectionKind::PersonMatte)
            .unwrap()
            .cache_key(),
        CorrectionRequest::new(asset, CorrectionKind::Stabilize)
            .unwrap()
            .cache_key()
    );
}

#[test]
fn a_missing_matte_renders_unmatted_and_export_refuses_until_the_analysis_is_stored() {
    let asset = fixture("shaky-240", 240, 135, 12);
    let (mut s, item) = session_over(asset.clone(), 12);
    // Background removal on, stabilization off: only the matte is needed.
    s.apply(Command::SetItemField {
        item,
        patch: engine::ItemParamPatch::Stabilize(None),
    })
    .unwrap();
    s.apply(Command::SetItemField {
        item,
        patch: engine::ItemParamPatch::BackgroundRemoval(Some(project::BackgroundRemoval {
            feather: 30.0,
        })),
    })
    .unwrap();
    let (mut runner, mut cache, _dir) = setup();
    let before = resolve(s.project(), &mut cache);
    assert_eq!(before.missing, vec![(item, CorrectionKind::PersonMatte)]);
    assert!(before.mattes.is_empty());
    let project = s.project().clone();
    let mut export = render::ExportSettings::from_project(&project);
    before.attach(&mut export.render);
    assert_export_refused(&project, export, &[item]);
    let request = CorrectionRequest::new(asset, CorrectionKind::PersonMatte).unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let job = request
        .submit(&mut runner, &mut cache, Some(brightness_segmenter(calls)))
        .unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    job.finish(&mut runner, &mut cache).unwrap().unwrap();
    let ready = resolve(s.project(), &mut cache);
    assert!(ready.missing.is_empty());
    assert!(ready.mattes.contains_key(&item));
    // A feather change keeps the key: only the item parameter differs.
    s.apply(Command::SetItemField {
        item,
        patch: engine::ItemParamPatch::BackgroundRemovalFeather(80.0),
    })
    .unwrap();
    assert!(resolve(s.project(), &mut cache).missing.is_empty());
}

#[test]
fn a_cancelled_matte_publishes_nothing() {
    let asset = fixture("shaky-240", 240, 135, 12);
    let (mut runner, mut cache, _dir) = setup();
    let request = CorrectionRequest::new(asset, CorrectionKind::PersonMatte).unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let job = request
        .submit(&mut runner, &mut cache, Some(brightness_segmenter(calls)))
        .unwrap();
    job.cancel.cancel();
    assert_eq!(wait(&mut runner, &job), JobState::Cancelled);
    assert!(job.finish(&mut runner, &mut cache).is_err());
    assert_eq!(request.cached(&mut cache).unwrap(), None);
}

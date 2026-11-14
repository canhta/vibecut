#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and assertions"
)]
//! Footage correction: stabilize, the person matte and flicker removal measured once per source,
//! looked up by key, applied at render.
use crate::support::control::request;
use engine::{
    ByteBudget, Command, CreatedId, RuntimeError, Session,
    cache::ArtifactCache,
    control::{Caller, ControlResult, MediaAccess, Policy, ResponseValue},
    footage::{
        Correction, CorrectionJob, CorrectionKind, CorrectionRequest, Lookup, Segmenter, resolve,
        uncorrected,
    },
    jobs::{JobRunner, JobState, RunnerConfig, SystemClock},
    luts::LutStore,
    recovery::{Autosave, Schedule},
    thumbnails::ThumbnailCache,
};
use media::{AssetId, SourceAsset};
use project::{ItemId, Project, ProjectId, StabilizeLevel, Thumbnail};
use render::stabilize::{Stabilizer, Track};
use serde_json::json;
use std::{
    io::Write,
    process::Stdio,
    sync::{Arc, Mutex},
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

/// Stabilize of `asset` analysed into a fresh cache, as a finished background job leaves it.
fn analysed(asset: &SourceAsset) -> (ArtifactCache, tempfile::TempDir) {
    let (mut runner, mut cache, dir) = setup();
    let request = CorrectionRequest::new(asset.clone(), CorrectionKind::Stabilize).unwrap();
    let job = request.submit(&mut runner, &mut cache, None).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    job.finish(&mut runner, &mut cache).unwrap().unwrap();
    (cache, dir)
}

/// Control admission with no policy limit; the media cache is the one lent to the process.
struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &engine::Command, _: &Project) -> engine::control::Result<()> {
        Ok(())
    }
}

#[test]
fn a_freeze_frame_of_a_stabilized_clip_equals_the_corrected_frame() {
    let asset = fixture("shaky-240", 240, 135, 12);
    let (mut s, _) = session_over(asset.clone(), 12);
    let (mut cache, dir) = analysed(&asset);
    // The frame a preview seek draws: the stored track attached to the render settings.
    let at = Tick(TICKS_PER_SECOND / 10);
    let resolution = resolve(s.project(), &mut cache);
    assert!(resolution.missing.is_empty(), "the analysis is stored");
    let corrected = engine::snapshot::image(s.project(), at, &resolution)
        .unwrap()
        .png;
    let uncorrected_frame = engine::snapshot::image(s.project(), at, &Default::default())
        .unwrap()
        .png;
    assert_ne!(
        corrected, uncorrected_frame,
        "the stored track changes the frame"
    );
    // Freeze runs through control, which reads the same lent cache the preview reads.
    let media = Mutex::new(MediaAccess::new(
        cache,
        ThumbnailCache::open(
            &dir.path().join("thumbnails"),
            ByteBudget::new(64 << 20).unwrap(),
            16,
        )
        .unwrap(),
        LutStore::open(&dir.path().join("luts")).unwrap(),
    ));
    let frozen = dir.path().join("frozen");
    let (mut jobs, _, _) = setup();
    let policy = Allow;
    let caller = Caller {
        principal: "test",
        external: false,
        policy: &policy,
        fonts: None,
        sounds: None,
        music: None,
        stickers: None,
        brand: None,
        media: Some(&media),
        chapters: None,
        captions: None,
        vision: None,
    };
    let req = request(
        &s,
        "timeline.freeze_frame",
        json!({"item": 10, "at": TICKS_PER_SECOND / 10, "duration": TICKS_PER_SECOND, "dir": frozen}),
        "freeze-stabilized",
    );
    let response = s.control_request(req, caller, &mut jobs).unwrap();
    let ResponseValue::Result(ControlResult::Edit { outcome }) = response.value else {
        panic!("freeze refused");
    };
    let CreatedId::Item(still) = outcome.created[0] else {
        panic!("still item id");
    };
    let still_asset = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == still)
        .and_then(|candidate| candidate.source_asset)
        .unwrap();
    let path = s
        .project()
        .data()
        .assets
        .iter()
        .find(|candidate| candidate.id() == still_asset)
        .unwrap()
        .path()
        .to_path_buf();
    assert_eq!(std::fs::read(&path).unwrap(), corrected);
}

#[test]
fn a_cover_of_a_stabilized_clip_draws_the_stored_correction() {
    let asset = fixture("shaky-240", 240, 135, 12);
    let (cache, _dir) = analysed(&asset);
    let shared = Arc::new(Mutex::new(cache));
    let lent: Lookup = {
        let shared = shared.clone();
        Arc::new(move |project: &Project| resolve(project, &mut shared.lock().unwrap()))
    };
    let cover = |lookup: Lookup| {
        let root = tempfile::tempdir().unwrap();
        let (session, _) = session_over(asset.clone(), 12);
        let mut autosave = Autosave::new(
            &root.path().join("recovery"),
            session,
            ByteBudget::new(512 << 20).unwrap(),
            lookup,
            Arc::new(SystemClock::default()),
            Schedule::default(),
        )
        .unwrap();
        autosave.apply(Command::ThumbnailProject {}).unwrap();
        match autosave.session().metadata().thumbnail.clone() {
            Thumbnail::Ready { rgba, .. } => rgba,
            _ => panic!("cover unavailable"),
        }
    };
    assert_ne!(
        cover(lent),
        cover(uncorrected()),
        "the cover draws the stored stabilize"
    );
}

/// Linear light of one pixel of `shot` before any gain: two shots are two unrelated textures
/// stretched to a wide contrast, scaled to leave headroom for the gain.
fn shot_light(shot: usize, x: u32, y: u32) -> f64 {
    let t = texture(
        f64::from(x) + 300.0 * shot as f64,
        f64::from(y) + 170.0 * shot as f64,
    );
    0.8 * (0.05 + 0.9 * ((t - 0.5) * 2.0 + 0.5).clamp(0.0, 1.0))
}
fn srgb_byte(light: f64) -> u8 {
    let encoded = if light <= 0.003_130_8 {
        12.92 * light
    } else {
        1.055 * light.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}
/// A 160x90 lossless clip of `frames` frames: frame `n` shows `light(n) = (shot, multiplier)`.
fn light_fixture(name: &str, frames: usize, light: impl Fn(usize) -> (usize, f64)) -> SourceAsset {
    let (width, height) = (160u32, 90u32);
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
        for n in 0..frames {
            let (shot, gain) = light(n);
            let mut pixels = Vec::with_capacity((width * height * 3) as usize);
            for y in 0..height {
                for x in 0..width {
                    let byte = srgb_byte(shot_light(shot, x, y) * gain);
                    pixels.extend_from_slice(&[byte; 3]);
                }
            }
            input.write_all(&pixels).unwrap();
        }
        drop(input);
        assert!(child.wait().unwrap().success());
    });
    SourceAsset::import(AssetId(1), path).unwrap()
}
fn analyze_flicker(asset: &SourceAsset) -> (Arc<render::flicker::FlickerTrack>, Vec<u8>) {
    let (mut runner, mut cache, _dir) = setup();
    let request = CorrectionRequest::new(asset.clone(), CorrectionKind::Flicker).unwrap();
    assert_eq!(request.cached(&mut cache).unwrap(), None);
    // Flicker needs no model: a cold analysis runs without a segmenter.
    let job = request.submit(&mut runner, &mut cache, None).unwrap();
    assert_eq!(
        wait(&mut runner, &job),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|s| s.error)
    );
    let stored = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(request.cached(&mut cache).unwrap(), Some(stored.clone()));
    let Correction::Flicker(track) = stored else {
        panic!("flicker track");
    };
    (track, runner.output_bytes(job.id).unwrap().to_vec())
}
/// Interior RMS and maximum, in percent, of `ln(observed * gain / truth)` with its constant
/// removed, excluding one 31-frame window at each end: the recorded oracle's metric, taken over
/// the means the analysis measured on the clean clip and on the flickering one.
fn residual(
    observed: &render::flicker::FlickerTrack,
    truth: &render::flicker::FlickerTrack,
    corrected: bool,
) -> (f64, f64) {
    let errors: Vec<f64> = (0..observed.len())
        .map(|n| {
            let gain = if corrected { observed.gain(n) } else { 1.0 };
            (f64::from(observed.mean(n).unwrap()) * f64::from(gain)
                / f64::from(truth.mean(n).unwrap()))
            .ln()
        })
        .collect();
    let interior = &errors[31..errors.len() - 31];
    let mean = interior.iter().sum::<f64>() / interior.len() as f64;
    let rms =
        (interior.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / interior.len() as f64).sqrt();
    let max = interior
        .iter()
        .map(|e| (e - mean).abs())
        .fold(0.0, f64::max);
    (100.0 * rms, 100.0 * max)
}
fn flicker_gain(hertz: f64, n: usize) -> f64 {
    1.0 + 0.08 * (std::f64::consts::TAU * hertz * n as f64 / FPS as f64).sin()
}
/// A shot whose brightness drifts slowly, as in the clean-up study's clean clip.
fn drift(n: usize) -> f64 {
    1.0 + 0.1428 * (std::f64::consts::TAU * n as f64 / FPS as f64 / 5.3).sin()
}
/// The recorded oracle bounds of the footage clean-up study (1 s Hann window): flicker fixtures
/// interior RMS and maximum, fade RMS, hard cut with the reset RMS, in percent.
const FLICKER_RMS: f64 = 0.6;
const FLICKER_MAX: f64 = 2.0;
const FADE_RMS: f64 = 0.6;
const CUT_RMS: f64 = 0.5;

#[test]
fn flicker_falls_below_the_recorded_bound_a_fade_survives_and_a_cut_does_not_pump() {
    let frames = 150;
    let clean = analyze_flicker(&light_fixture("flicker-clean", frames, |n| (0, drift(n)))).0;
    // 100 Hz mains aliased by 30 fps to a 2.97-frame period, and the 17.6-frame beat.
    for (hertz, name) in [(10.09, "flicker-mains"), (1.7, "flicker-beat")] {
        let asset = light_fixture(name, frames, |n| (0, drift(n) * flicker_gain(hertz, n)));
        let (observed, _) = analyze_flicker(&asset);
        let (rms, max) = residual(&observed, &clean, true);
        let before = residual(&observed, &clean, false).0;
        println!("FLICKER {hertz} Hz: rms {rms:.3} % max {max:.3} % (before {before:.2} %)");
        assert!(
            rms <= FLICKER_RMS && max <= FLICKER_MAX,
            "{hertz} Hz {rms} {max}"
        );
        assert!(
            (0..frames).all(|n| !observed.is_cut(n)),
            "no cut in one shot"
        );
        // The frame-to-frame variation was an order of magnitude larger before the correction.
        assert!(before > 10.0 * rms, "{hertz} Hz before {before}");
    }
    // A fade to 40 % under the mains flicker: the fade is not flattened.
    let fade = |n: usize| drift(n) * (1.0 - 0.12 * n as f64 / FPS as f64);
    let fade_clean = analyze_flicker(&light_fixture("flicker-fade-clean", frames, |n| {
        (0, fade(n))
    }))
    .0;
    let faded = light_fixture("flicker-fade", frames, |n| {
        (0, fade(n) * flicker_gain(10.09, n))
    });
    let (fading, _) = analyze_flicker(&faded);
    let (rms, max) = residual(&fading, &fade_clean, true);
    println!("FLICKER fade: rms {rms:.3} % max {max:.3} %");
    assert!(rms <= FADE_RMS, "fade {rms}");
    let corrected = |n: usize| f64::from(fading.mean(n).unwrap()) * f64::from(fading.gain(n));
    assert!(
        corrected(120) < 0.7 * corrected(30),
        "the fade survives the correction"
    );
    // Two shots of unrelated pictures and brightness, flickering at 10.09 Hz, cut at frame 75.
    let level = |n: usize| {
        let base = if n < 75 { 0.30 } else { 0.55 };
        (base + 0.02 * (std::f64::consts::TAU * n as f64 / FPS as f64 / 6.0).sin()) / 0.55
    };
    let shot = |n: usize| usize::from(n >= 75);
    let cut_clean = analyze_flicker(&light_fixture("flicker-cut-clean", frames, |n| {
        (shot(n), level(n))
    }))
    .0;
    let cut = light_fixture("flicker-cut", frames, |n| {
        (shot(n), level(n) * flicker_gain(10.09, n))
    });
    let (cutting, _) = analyze_flicker(&cut);
    // The shot change is found at its first frame and nowhere else, flicker or not.
    assert!((0..frames).all(|n| cutting.is_cut(n) == (n == 75)));
    assert!((0..frames).all(|n| cut_clean.is_cut(n) == (n == 75)));
    let (rms, max) = residual(&cutting, &cut_clean, true);
    println!("FLICKER cut: rms {rms:.3} % max {max:.3} %");
    assert!(rms <= CUT_RMS, "cut {rms}");
}

#[test]
fn two_uncached_flicker_runs_retain_the_same_bytes_and_the_kinds_never_share_a_key() {
    let asset = light_fixture("flicker-mains", 150, |n| {
        (0, drift(n) * flicker_gain(10.09, n))
    });
    let (first, first_bytes) = analyze_flicker(&asset);
    let (second, second_bytes) = analyze_flicker(&asset);
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_bytes, first.encode());
    assert_eq!(
        *second,
        render::flicker::FlickerTrack::decode(&first_bytes).unwrap()
    );
    assert_eq!(first.len(), 150);
    // The flicker really moves the measured means, so equal bytes are not equal constants.
    assert!(first.mean(0) != first.mean(1));
    let key = |kind| {
        CorrectionRequest::new(asset.clone(), kind)
            .unwrap()
            .cache_key()
    };
    assert_ne!(key(CorrectionKind::Flicker), key(CorrectionKind::Stabilize));
    assert_ne!(
        key(CorrectionKind::Flicker),
        key(CorrectionKind::PersonMatte)
    );
}

#[test]
fn a_cleared_cache_needs_flicker_analysis_and_export_refuses_but_split_keeps_the_track() {
    let asset = light_fixture("flicker-mains", 150, |n| {
        (0, drift(n) * flicker_gain(10.09, n))
    });
    let (mut s, item) = session_over(asset.clone(), 150);
    s.apply(Command::SetItemField {
        item,
        patch: engine::ItemParamPatch::Stabilize(None),
    })
    .unwrap();
    s.apply(Command::SetItemField {
        item,
        patch: engine::ItemParamPatch::RemoveFlicker(true),
    })
    .unwrap();
    let (mut runner, mut cache, _dir) = setup();
    let before = resolve(s.project(), &mut cache);
    assert_eq!(before.missing, vec![(item, CorrectionKind::Flicker)]);
    assert!(before.flicker.is_empty());
    let project = s.project().clone();
    let mut export = render::ExportSettings::from_project(&project);
    before.attach(&mut export.render);
    assert_export_refused(&project, export, &[item]);
    let request = CorrectionRequest::new(asset, CorrectionKind::Flicker).unwrap();
    let job = request.submit(&mut runner, &mut cache, None).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    job.finish(&mut runner, &mut cache).unwrap().unwrap();
    let ready = resolve(s.project(), &mut cache);
    assert!(ready.missing.is_empty());
    assert!(ready.flicker.contains_key(&item));
    // A split needs no new analysis: both halves find the same stored track.
    s.apply(Command::SplitAt {
        item,
        at: Tick(TICKS_PER_SECOND),
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
    assert!(Arc::ptr_eq(
        &split.flicker[&ids[0]],
        &split.flicker[&ids[1]]
    ));
    // A cleared cache is Needs analysis again for both halves.
    let (_runner, mut empty, _other) = setup();
    assert_eq!(
        resolve(s.project(), &mut empty).missing,
        ids.iter()
            .map(|id| (*id, CorrectionKind::Flicker))
            .collect::<Vec<_>>()
    );
}

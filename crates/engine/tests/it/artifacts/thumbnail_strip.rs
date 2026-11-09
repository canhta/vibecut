#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and explicit assertions"
)]
use engine::{
    ByteBudget, EditError,
    jobs::{Clock, JobRunner, JobState, RunnerConfig, SystemClock},
    thumbnail_strip::{
        MAX_TILES, ThumbnailStripRequest, ThumbnailStripState, display, tile_interval,
    },
    thumbnails::ThumbnailCache,
};
use media::{AssetId, SourceAsset, THUMBNAIL_MAX_EDGE};
use project::{Item, ItemId, ItemParams, Project, ProjectId, Retime, Track, TrackId, TrackKind};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{SourceRange, Speed, TICKS_PER_SECOND, Tick, TimeRange};

fn t(seconds: i64) -> Tick {
    Tick(seconds * TICKS_PER_SECOND)
}
fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(t(start), t(end)).unwrap()
}
fn window(start: i64, end: i64) -> TimeRange {
    TimeRange::new(t(start), t(end)).unwrap()
}

/// 3 s of 10 fps video with a moving pattern.
fn clip() -> Option<PathBuf> {
    if !testkit::oracle::require("thumbnail strip") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("thumbnail-strip"),
        "thumbnail strip",
        "clip.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x36:r=10:d=3",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p",
        ],
    )
}

/// Different content, same duration: a changed source hash.
fn changed_clip() -> Option<PathBuf> {
    if !testkit::oracle::require("thumbnail strip") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("thumbnail-strip"),
        "thumbnail strip",
        "changed.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=64x36:r=10:d=3",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p",
        ],
    )
}

fn audio_only() -> Option<PathBuf> {
    if !testkit::oracle::require("thumbnail strip") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("thumbnail-strip"),
        "thumbnail strip",
        "tone.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=8000:duration=1",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

fn still() -> Option<PathBuf> {
    if !testkit::oracle::require("thumbnail strip") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("thumbnail-strip"),
        "thumbnail strip",
        "still.png",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=green:s=64x36:d=1",
            "-frames:v",
            "1",
        ],
    )
}

fn source_asset(path: &Path, id: u64) -> SourceAsset {
    SourceAsset::import(AssetId(id), path.to_path_buf()).unwrap()
}

fn item(asset: &SourceAsset, source: SourceRange, start: Tick, speed: f64) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Retime {
            speed: Speed::from_f64(speed).unwrap(),
            ..Default::default()
        },
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(1),
        source_asset: Some(asset.id()),
        source_range: source,
        start,
        content: ItemParams::Video(project::VideoParams {
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
            fades: Default::default(),
        }),
    }
}

fn setup() -> (JobRunner, ThumbnailCache, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (runner, cache) = setup_at(dir.path(), budget, Arc::new(SystemClock::default()));
    (runner, cache, dir)
}

fn setup_at(root: &Path, budget: ByteBudget, clock: Arc<dyn Clock>) -> (JobRunner, ThumbnailCache) {
    let cache = ThumbnailCache::open(root, budget.clone(), 64).unwrap();
    let runner = JobRunner::new(
        RunnerConfig {
            concurrency: [2; 3],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock,
    )
    .unwrap();
    (runner, cache)
}

fn entries(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() != ".cache.lock")
        .count()
}

fn wait(runner: &mut JobRunner, id: engine::jobs::JobId) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return state;
        }
        assert!(Instant::now() < deadline, "thumbnail strip timeout");
        std::thread::yield_now();
    }
}

fn run(
    request: &ThumbnailStripRequest,
) -> (
    engine::thumbnail_strip::ThumbnailStrip,
    JobRunner,
    ThumbnailCache,
    tempfile::TempDir,
) {
    let (mut runner, mut cache, dir) = setup();
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(
        wait(&mut runner, job.id),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|snapshot| snapshot.error)
    );
    let strip = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    (strip, runner, cache, dir)
}

fn ready(state: ThumbnailStripState) -> engine::thumbnail_strip::ThumbnailStripDisplay {
    match state {
        ThumbnailStripState::Ready(display) => display,
        other => panic!("expected ready tiles, got {other:?}"),
    }
}

#[test]
fn trim_and_retime_map_to_source_frames() {
    let Some(path) = clip() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    assert_eq!(request.positions(), &[t(0), t(1), t(2)]);
    let (strip, _runner, mut cache, _dir) = run(&request);
    assert_eq!(strip.positions(), &[t(0), t(1), t(2)]);

    // Trim: source 1..3 s starts at timeline zero.
    let trimmed = item(&asset, range(1, 3), Tick::ZERO, 1.0);
    let tiles = ready(
        display(
            &asset,
            &trimmed,
            window(0, 2),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache,
        )
        .unwrap(),
    );
    assert_eq!(tiles.tiles().len(), 2);
    assert_eq!(tiles.tiles()[0].at, t(1));
    assert_eq!(tiles.tiles()[0].output, window(0, 1));
    assert_eq!(tiles.tiles()[1].at, t(2));
    assert_eq!(tiles.tiles()[1].output, window(1, 2));

    // Retime 2x: 3 s of source spans 1.5 s of timeline.
    let retimed = item(&asset, range(0, 3), Tick::ZERO, 2.0);
    let tiles = ready(
        display(
            &asset,
            &retimed,
            TimeRange::new(Tick::ZERO, Tick(3 * TICKS_PER_SECOND / 2)).unwrap(),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache,
        )
        .unwrap(),
    );
    assert_eq!(tiles.tiles().len(), 2);
    assert_eq!(tiles.tiles()[0].at, t(0));
    assert_eq!(tiles.tiles()[1].at, t(2));

    // Trim and retime together: source 1..3 s at 2x spans 1 s of timeline.
    let both = item(&asset, range(1, 3), Tick::ZERO, 2.0);
    let tiles = ready(
        display(
            &asset,
            &both,
            window(0, 1),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache,
        )
        .unwrap(),
    );
    assert_eq!(tiles.tiles().len(), 1);
    assert_eq!(tiles.tiles()[0].at, t(1));

    // A later clip start moves the same source window down the timeline.
    let moved = item(&asset, range(1, 3), t(5), 1.0);
    let tiles = ready(
        display(
            &asset,
            &moved,
            window(5, 7),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache,
        )
        .unwrap(),
    );
    assert_eq!(tiles.tiles()[0].at, t(1));
    assert_eq!(tiles.tiles()[1].at, t(2));
    assert!(matches!(
        display(
            &asset,
            &moved,
            window(8, 9),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        ),
        Err(EditError::Invalid("thumbnail strip display outside clip"))
    ));
}

#[test]
fn cold_pending_ready_and_hostile_inputs() {
    let Some(path) = clip() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    let (_runner, mut cache, _dir) = setup();
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Pending
    ));
    for interval in [Tick::ZERO, Tick(-1)] {
        assert!(matches!(
            ThumbnailStripRequest::new(asset.clone(), range(0, 3), interval, THUMBNAIL_MAX_EDGE),
            Err(EditError::Invalid("thumbnail strip interval"))
        ));
        assert!(matches!(
            display(
                &asset,
                &clip,
                window(0, 3),
                interval,
                THUMBNAIL_MAX_EDGE,
                &mut cache
            ),
            Err(EditError::Invalid("thumbnail strip interval"))
        ));
    }
    for edge in [0, THUMBNAIL_MAX_EDGE + 1] {
        assert!(matches!(
            ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), edge),
            Err(EditError::Invalid("thumbnail max edge"))
        ));
        assert!(matches!(
            display(&asset, &clip, window(0, 3), t(1), edge, &mut cache),
            Err(EditError::Invalid("thumbnail max edge"))
        ));
    }
    assert!(matches!(
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), Tick(1), THUMBNAIL_MAX_EDGE),
        Err(EditError::Invalid("thumbnail strip tiles"))
    ));
    let other = source_asset(&path, 2);
    assert!(matches!(
        display(
            &asset,
            &item(&other, range(0, 3), Tick::ZERO, 1.0),
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        ),
        Err(EditError::Invalid("thumbnail strip display asset"))
    ));
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(4, 5),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        ),
        Err(EditError::Invalid("thumbnail strip display outside clip"))
    ));
    let long = item(&asset, range(0, 300), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &asset,
            &long,
            window(0, 300),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        ),
        Err(EditError::Invalid("thumbnail strip tiles"))
    ));

    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    let (_strip, _runner, mut cache, _dir) = run(&request);
    let strip = ready(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache,
        )
        .unwrap(),
    );
    assert_eq!(strip.tiles().len(), 3);
    assert_eq!(strip.tiles()[2].at, t(2));

    let Some(audio) = audio_only() else {
        return;
    };
    let audio = source_asset(&audio, 3);
    assert!(matches!(
        ThumbnailStripRequest::new(audio.clone(), range(0, 1), t(1), THUMBNAIL_MAX_EDGE),
        Err(EditError::Unsupported("thumbnail strip requires video"))
    ));
    assert!(matches!(
        display(
            &audio,
            &item(&audio, range(0, 1), Tick::ZERO, 1.0),
            window(0, 1),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Unsupported
    ));
}

#[test]
fn still_images_repeat_one_cached_frame() {
    let Some(path) = still() else {
        return;
    };
    let asset = source_asset(&path, 1);
    assert!(asset.is_image());
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 1), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    assert_eq!(request.positions(), &[Tick::ZERO]);
    let (_strip, _runner, mut cache, _dir) = run(&request);
    let clip = item(&asset, range(0, 1), Tick::ZERO, 1.0);
    let tiles = ready(
        display(
            &asset,
            &clip,
            window(0, 1),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache,
        )
        .unwrap(),
    );
    assert_eq!(tiles.tiles().len(), 1);
    assert_eq!(tiles.tiles()[0].at, Tick::ZERO);
    assert!(tiles.tiles()[0].image.width > 0);
}

#[test]
fn warm_hit_opens_no_source_and_changed_content_misses() {
    let Some(source) = clip() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("clip.mkv");
    std::fs::copy(&source, &path).unwrap();
    let asset = source_asset(&path, 1);
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    let (first, mut runner, mut cache, _dir) = run(&request);
    assert_eq!(first.positions().len(), 3);

    let hidden = path.with_extension("hidden");
    std::fs::rename(&path, &hidden).unwrap();
    let again = request.submit(&mut runner, &mut cache).unwrap();
    let state = wait(&mut runner, again.id);
    std::fs::rename(&hidden, &path).unwrap();
    assert_eq!(state, JobState::Completed);
    let strip = again.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(strip.positions(), first.positions());
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Ready(_)
    ));

    let Some(changed_path) = changed_clip() else {
        return;
    };
    let changed = source_asset(&changed_path, 2);
    let changed_request =
        ThumbnailStripRequest::new(changed.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    let changed_clip_item = item(&changed, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &changed,
            &changed_clip_item,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Pending
    ));
    assert_ne!(changed.content_hash(), asset.content_hash());
    assert_eq!(changed_request.positions(), request.positions());
}

#[test]
fn deleted_source_fails_and_keeps_no_frames() {
    let Some(source) = clip() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("gone.mkv");
    std::fs::copy(&source, &path).unwrap();
    let asset = source_asset(&path, 1);
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    std::fs::remove_file(&path).unwrap();
    let (mut runner, mut cache, dir) = setup();
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Failed);
    assert!(matches!(
        job.finish(&mut runner, &mut cache),
        Err(EditError::JobFailure(_))
    ));
    assert_eq!(entries(dir.path()), 0);
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Pending
    ));
}

struct FrameGate {
    arrived: mpsc::SyncSender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
    first: AtomicBool,
}
impl Clock for FrameGate {
    fn now(&self) -> Duration {
        if std::thread::current().name() == Some("vibecut-job")
            && !self.first.swap(true, Ordering::SeqCst)
        {
            self.arrived.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30))
                .unwrap();
        }
        Duration::from_secs(1)
    }
}

#[test]
fn cancelling_a_changed_source_keeps_the_previous_cache() {
    let Some(path) = clip() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    let (_first, old_runner, old_cache, dir) = run(&request);
    assert_eq!(entries(dir.path()), 3);
    drop(old_cache);
    drop(old_runner);

    let Some(changed_path) = changed_clip() else {
        return;
    };
    let changed = source_asset(&changed_path, 2);
    let (arrived, arrived_rx) = mpsc::sync_channel(1);
    let (resume, resume_rx) = mpsc::sync_channel(1);
    let clock = Arc::new(FrameGate {
        arrived,
        resume: Mutex::new(resume_rx),
        first: AtomicBool::new(false),
    });
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (mut runner, mut cache) = setup_at(dir.path(), budget, clock);
    assert_eq!(entries(dir.path()), 3, "reopened cache keeps old frames");
    let job = ThumbnailStripRequest::new(changed.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE)
        .unwrap()
        .submit(&mut runner, &mut cache)
        .unwrap();
    runner.tick().unwrap();
    arrived_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(runner.snapshot(job.id).unwrap().state, JobState::Running);
    job.cancel.cancel();
    resume.send(()).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Cancelled);
    assert!(matches!(
        job.finish(&mut runner, &mut cache),
        Err(EditError::Cancelled)
    ));
    assert!(runner.output_bytes(job.id).is_none());
    assert_eq!(
        entries(dir.path()),
        3,
        "cancel leaves the previous frames intact"
    );

    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Ready(_)
    ));
    let changed_clip_item = item(&changed, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &changed,
            &changed_clip_item,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Pending
    ));
    assert_eq!(entries(dir.path()), 3);
}

#[test]
fn corrupt_cached_frames_are_failed_and_regenerate() {
    let Some(path) = clip() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    let (first, mut runner, mut cache, dir) = run(&request);
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != ".cache.lock" {
            // Keep the envelope header valid; the payload checksum must fail.
            let mut bytes = std::fs::read(entry.path()).unwrap();
            if let Some(last) = bytes.last_mut() {
                *last ^= 0xff;
            }
            std::fs::write(entry.path(), bytes).unwrap();
        }
    }
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Failed(EditError::CacheCorrupt)
    ));
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let strip = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(strip.positions(), first.positions());
    assert!(matches!(
        display(
            &asset,
            &clip,
            window(0, 3),
            t(1),
            THUMBNAIL_MAX_EDGE,
            &mut cache
        )
        .unwrap(),
        ThumbnailStripState::Ready(_)
    ));
}

#[test]
fn project_json_stays_byte_identical_after_generation() {
    let Some(path) = clip() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let mut data = Project::empty(ProjectId(1), "Thumbnail strip".into())
        .unwrap()
        .into_data();
    data.assets.push(asset.clone());
    data.scene_mut().timeline.main = Track {
        id: TrackId(1),
        name: "Video".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![item(&asset, range(0, 3), Tick::ZERO, 1.0)],
    };
    let project = Project::new(data).unwrap();
    let before = project::encode(&project, false).unwrap();
    let request =
        ThumbnailStripRequest::new(asset.clone(), range(0, 3), t(1), THUMBNAIL_MAX_EDGE).unwrap();
    let (_strip, _runner, _cache, _dir) = run(&request);
    let after = project::encode(&project, false).unwrap();
    assert_eq!(before, after);
}

#[test]
fn tile_interval_is_one_sixteen_by_nine_tile() {
    let interval = tile_interval(39, 71.5).unwrap();
    assert!(interval > Tick(TICKS_PER_SECOND / 2));
    assert!(interval < Tick(TICKS_PER_SECOND));
    assert!(tile_interval(39, 71.5).is_ok());
}

/// Manual check against a real 30-minute interview. Not part of CI: the sample
/// is local and may be absent. Reports generation time; peak RSS is measured by
/// the caller around this process.
#[test]
#[ignore = "manual: set VIBECUT_INTERVIEW_SAMPLE to interview-30m.mp4"]
fn interview_samples_one_visible_range() {
    let Some(path) = std::env::var_os("VIBECUT_INTERVIEW_SAMPLE").map(PathBuf::from) else {
        eprintln!("skipping: VIBECUT_INTERVIEW_SAMPLE is not set");
        return;
    };
    if !path.is_file() {
        eprintln!("skipping: {} does not exist", path.display());
        return;
    }
    let asset = SourceAsset::import(AssetId(30), path).unwrap();
    let duration = asset.probe().duration.unwrap();
    let clip = item(
        &asset,
        SourceRange::new(Tick::ZERO, duration).unwrap(),
        Tick::ZERO,
        1.0,
    );
    // One visible minute at the measured ruler scale (71.5 px/s) with a 39 px
    // filmstrip: one 16:9 tile every 39 * 16/9 / 71.5 s.
    let interval = tile_interval(39, 71.5).unwrap();
    let visible = window(0, 60);
    let request = ThumbnailStripRequest::new(
        asset.clone(),
        SourceRange::new(visible.start(), visible.end()).unwrap(),
        interval,
        THUMBNAIL_MAX_EDGE,
    )
    .unwrap();
    assert!(request.positions().len() <= MAX_TILES);
    let started = Instant::now();
    let (mut runner, mut cache, _dir) = setup();
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let strip = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    let generated = started.elapsed();
    let state = display(
        &asset,
        &clip,
        visible,
        interval,
        THUMBNAIL_MAX_EDGE,
        &mut cache,
    )
    .unwrap();
    assert!(matches!(state, ThumbnailStripState::Ready(_)));
    println!(
        "thumbnail strip interview: positions={} interval_ticks={} visible={visible:?} generated={generated:?}",
        strip.positions().len(),
        interval.0
    );
}

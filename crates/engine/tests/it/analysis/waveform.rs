#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and explicit assertions"
)]
use engine::{
    AnalysisError, ByteBudget,
    cache::ArtifactCache,
    jobs::{Clock, JobRunner, JobState, RunnerConfig, SystemClock},
    waveform::{MAX_COLUMNS, WaveformRequest, WaveformState, display},
};
use media::{AssetId, SourceAsset};
use project::{
    AudioParams, Item, ItemId, ItemParams, Project, ProjectId, Retime, Track, TrackId, TrackKind,
};
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
fn one_and_a_half() -> TimeRange {
    TimeRange::new(Tick::ZERO, Tick(3 * TICKS_PER_SECOND / 2)).unwrap()
}

/// 3 s of a 200 Hz tone with 0.1, 0.5 and 0.9 amplitude sections, one per second.
fn sections() -> Option<PathBuf> {
    if !testkit::oracle::require("waveform peaks") {
        return None;
    }
    let expression = "(0.1*between(t\\,0\\,1)+0.5*between(t\\,1\\,2)+0.9*between(t\\,2\\,3))*\
        sin(2*PI*200*t)";
    let input = format!("aevalsrc={expression}:s=8000:d=3");
    testkit::oracle::generate(
        &testkit::fixture::dir("waveform"),
        "waveform peaks",
        "sections.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            input.as_str(),
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

/// 0.5 s of 1.2 amplitude, past 0 dBFS.
fn loud() -> Option<PathBuf> {
    if !testkit::oracle::require("waveform peaks") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("waveform"),
        "waveform peaks",
        "loud.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=1.2*sin(2*PI*440*t):s=8000:d=0.5",
            "-ac",
            "1",
            "-c:a",
            "pcm_f32le",
        ],
    )
}

fn stereo() -> Option<PathBuf> {
    if !testkit::oracle::require("waveform peaks") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("waveform"),
        "waveform peaks",
        "stereo.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.2*sin(2*PI*200*t)|0.8*sin(2*PI*300*t):s=8000:d=1",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

fn video_only(dir: &Path) -> SourceAsset {
    let path = testkit::oracle::generate(
        dir,
        "waveform peaks",
        "video-only.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=black:s=32x32:r=10:d=1",
            "-c:v",
            "ffv1",
        ],
    )
    .unwrap();
    SourceAsset::import(AssetId(9), path).unwrap()
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
        content: ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
            cleanup: Default::default(),
        }),
    }
}

fn setup() -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (runner, cache) = setup_at(dir.path(), Arc::new(SystemClock::default()));
    (runner, cache, dir)
}

fn setup_at(root: &Path, clock: Arc<dyn Clock>) -> (JobRunner, ArtifactCache) {
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (cache, _) = ArtifactCache::open(root, budget.clone(), 16).unwrap();
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
        assert!(Instant::now() < deadline, "waveform timeout");
        std::thread::yield_now();
    }
}

fn run(
    request: &WaveformRequest,
) -> (
    media::PeakSummary,
    JobRunner,
    ArtifactCache,
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
    let summary = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    (summary, runner, cache, dir)
}

fn ready_bars(state: WaveformState) -> Vec<engine::waveform::WaveformBar> {
    match state {
        WaveformState::Ready(display) => display.bars().to_vec(),
        other => panic!("expected ready peaks, got {other:?}"),
    }
}

fn peaks(bars: &[engine::waveform::WaveformBar]) -> Vec<f32> {
    bars.iter().map(|bar| bar.max.max(-bar.min)).collect()
}

#[test]
fn trim_and_retime_map_to_bucket_positions() {
    let Some(path) = sections() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let (summary, _runner, mut cache, _dir) = run(&request);
    assert_eq!(summary.sample_rate(), 8_000);
    assert_eq!(summary.channels(), 1);
    assert_eq!(summary.total_samples(), 24_000);
    assert_eq!(
        summary.bucket_count(),
        24_000_usize.div_ceil(media::PEAK_BUCKET_SAMPLES as usize)
    );

    // Trim: source 1..3 s starts at timeline zero.
    let trimmed = item(&asset, range(1, 3), Tick::ZERO, 1.0);
    let bars = ready_bars(display(&asset, &trimmed, window(0, 2), 8, &mut cache).unwrap());
    assert_eq!(bars.len(), 8);
    for (index, peak) in peaks(&bars).into_iter().enumerate() {
        let expected = if index < 4 { 0.5 } else { 0.9 };
        assert!(
            (peak - expected).abs() < 0.03,
            "trim bar {index}: {peak} vs {expected}"
        );
        assert!(!bars[index].over);
    }

    // Retime 2x: 3 s of source spans 1.5 s of timeline. Bars that meet a
    // section boundary share its 16 ms bucket, so assert the section centres.
    let retimed = item(&asset, range(0, 3), Tick::ZERO, 2.0);
    let bars = ready_bars(display(&asset, &retimed, one_and_a_half(), 12, &mut cache).unwrap());
    assert_eq!(bars.len(), 12);
    let values = peaks(&bars);
    for (index, expected) in [(1, 0.1), (5, 0.5), (9, 0.9)] {
        assert!(
            (values[index] - expected).abs() < 0.03,
            "retime bar {index}: {} vs {expected}",
            values[index]
        );
    }
    assert!((values[2] - 0.1).abs() < 0.03);
    assert!((values[4] - 0.5).abs() < 0.03);
    assert!((values[8] - 0.9).abs() < 0.03);

    // Trim and retime together: source 1..3 s at 2x spans 1 s of timeline.
    let both = item(&asset, range(1, 3), Tick::ZERO, 2.0);
    let bars = ready_bars(display(&asset, &both, window(0, 1), 8, &mut cache).unwrap());
    let values = peaks(&bars);
    assert!(values.iter().take(4).all(|peak| (*peak - 0.5).abs() < 0.03));
    assert!(values.iter().skip(4).all(|peak| (*peak - 0.9).abs() < 0.03));

    // A later clip start moves the same source window down the timeline.
    let moved = item(&asset, range(1, 3), t(5), 1.0);
    let before = display(&asset, &moved, window(8, 9), 1, &mut cache);
    assert!(matches!(
        before,
        Err(AnalysisError::Edit(engine::EditError::Invalid(
            "waveform display outside clip"
        )))
    ));
    let shifted = ready_bars(display(&asset, &moved, window(5, 7), 4, &mut cache).unwrap());
    assert_eq!(shifted.len(), 4);
    let values = peaks(&shifted);
    assert!(values.iter().take(2).all(|peak| (*peak - 0.5).abs() < 0.03));
    assert!(values.iter().skip(2).all(|peak| (*peak - 0.9).abs() < 0.03));
}

#[test]
fn cold_pending_ready_over_and_unsupported_states() {
    let Some(path) = sections() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let (mut runner, mut cache, _dir) = setup();
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&asset, &clip, window(0, 3), 8, &mut cache).unwrap(),
        WaveformState::Pending
    ));
    for columns in [0, MAX_COLUMNS + 1] {
        assert!(matches!(
            display(&asset, &clip, window(0, 3), columns, &mut cache),
            Err(AnalysisError::Edit(engine::EditError::Invalid(
                "waveform display columns"
            )))
        ));
    }
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert!(matches!(
        display(&asset, &clip, window(0, 3), 8, &mut cache).unwrap(),
        WaveformState::Ready(_)
    ));

    let Some(loud_path) = loud() else {
        return;
    };
    let loud_asset = source_asset(&loud_path, 2);
    let request = WaveformRequest::new(loud_asset.clone()).unwrap();
    let (summary, _runner, mut loud_cache, _dir) = run(&request);
    assert!((0..summary.bucket_count()).any(|bucket| summary.over(bucket)));
    let clip = item(&loud_asset, range(0, 1), Tick::ZERO, 1.0);
    let half = TimeRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND / 2)).unwrap();
    let bars = ready_bars(display(&loud_asset, &clip, half, 4, &mut loud_cache).unwrap());
    assert!(bars.iter().all(|bar| bar.over));
    let dir = path.parent().unwrap().to_path_buf();
    let video = video_only(&dir);
    assert!(matches!(
        WaveformRequest::new(video.clone()),
        Err(AnalysisError::Edit(engine::EditError::Unsupported(_)))
    ));
    let clip = item(&video, range(0, 1), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&video, &clip, window(0, 1), 4, &mut cache).unwrap(),
        WaveformState::Unsupported
    ));
}

fn synthetic(hash: u8, rate: u32, channels: u16) -> SourceAsset {
    testkit::source::asset(1, std::env::temp_dir().join("vibecut-waveform.mp4"))
        .format("mp4")
        .duration(t(1))
        .hash([hash; 32])
        .stream(
            testkit::source::audio()
                .time_base(media::TimeBase::new(1, 48_000).unwrap())
                .sample_rate(rate)
                .channels(channels)
                .duration(t(1))
                .build(),
        )
        .build()
}

#[test]
fn cache_key_binds_content_rate_channel_layout_and_recipe() {
    let base = WaveformRequest::new(synthetic(1, 48_000, 2))
        .unwrap()
        .cache_key();
    assert_eq!(
        base,
        WaveformRequest::new(synthetic(1, 48_000, 2))
            .unwrap()
            .cache_key()
    );
    for other in [
        WaveformRequest::new(synthetic(2, 48_000, 2)),
        WaveformRequest::new(synthetic(1, 44_100, 2)),
        WaveformRequest::new(synthetic(1, 48_000, 1)),
    ] {
        assert_ne!(base, other.unwrap().cache_key());
    }

    let video = testkit::source::asset(2, std::env::temp_dir().join("vibecut-waveform-video.mp4"))
        .format("mp4")
        .duration(t(1))
        .hash([3; 32])
        .stream(testkit::source::video().duration(t(1)).build())
        .build();
    assert!(matches!(
        WaveformRequest::new(video),
        Err(AnalysisError::Edit(engine::EditError::Unsupported(_)))
    ));
}

#[test]
fn warm_hit_opens_no_source_and_changed_content_misses() {
    let Some(source) = sections() else {
        return;
    };
    // The hidden-source step below must not borrow the fixture every other test
    // in this binary opens concurrently.
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("sections.wav");
    std::fs::copy(&source, &path).unwrap();
    let asset = source_asset(&path, 1);
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let (first, mut runner, mut cache, _dir) = run(&request);
    assert_eq!(cache.len(), 1);

    let hidden = path.with_extension("hidden");
    std::fs::rename(&path, &hidden).unwrap();
    let again = request.submit(&mut runner, &mut cache).unwrap();
    let state = wait(&mut runner, again.id);
    std::fs::rename(&hidden, &path).unwrap();
    assert_eq!(state, JobState::Completed);
    assert_eq!(
        again.finish(&mut runner, &mut cache).unwrap().unwrap(),
        first
    );
    assert_eq!(cache.len(), 1);

    let Some(loud_path) = loud() else {
        return;
    };
    let changed = source_asset(&loud_path, 2);
    let request = WaveformRequest::new(changed.clone()).unwrap();
    let clip = item(&changed, range(0, 1), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&changed, &clip, window(0, 1), 4, &mut cache).unwrap(),
        WaveformState::Pending
    ));
    let (changed_summary, _runner, _cache, _dir) = run(&request);
    assert_ne!(changed_summary.total_samples(), first.total_samples());
    assert_ne!(
        request.cache_key(),
        WaveformRequest::new(asset).unwrap().cache_key()
    );
}

#[test]
fn stereo_display_combines_channels_and_reports_over() {
    let Some(path) = stereo() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let (summary, _runner, mut cache, _dir) = run(&request);
    assert_eq!(summary.channels(), 2);
    let clip = item(&asset, range(0, 1), Tick::ZERO, 1.0);
    let bars = ready_bars(display(&asset, &clip, window(0, 1), 4, &mut cache).unwrap());
    for bar in &bars {
        assert!((bar.max - 0.8).abs() < 0.03, "{bar:?}");
        assert!((bar.min + 0.8).abs() < 0.03, "{bar:?}");
        assert!(!bar.over);
    }
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
    let Some(path) = sections() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let (first, old_runner, old_cache, dir) = run(&WaveformRequest::new(asset.clone()).unwrap());
    assert_eq!(first.total_samples(), 24_000);
    assert_eq!(old_cache.len(), 1);
    drop(old_cache);
    drop(old_runner);

    let Some(loud_path) = loud() else {
        return;
    };
    let changed = source_asset(&loud_path, 2);
    let (arrived, arrived_rx) = mpsc::sync_channel(1);
    let (resume, resume_rx) = mpsc::sync_channel(1);
    let clock = Arc::new(FrameGate {
        arrived,
        resume: Mutex::new(resume_rx),
        first: AtomicBool::new(false),
    });
    let (mut runner, mut cache) = setup_at(dir.path(), clock);
    assert_eq!(cache.len(), 1, "reopened cache keeps the old source entry");
    let job = WaveformRequest::new(changed.clone())
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
        Err(AnalysisError::Runtime(engine::RuntimeError::Cancelled))
    ));
    assert!(runner.output_bytes(job.id).is_none());
    assert_eq!(cache.len(), 1, "cancel leaves the previous entry intact");

    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&asset, &clip, window(0, 3), 8, &mut cache).unwrap(),
        WaveformState::Ready(_)
    ));
    let changed_clip = item(&changed, range(0, 1), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&changed, &changed_clip, window(0, 1), 4, &mut cache).unwrap(),
        WaveformState::Pending
    ));
    assert_eq!(cache.len(), 1);
}

#[test]
fn corrupt_cache_is_failed_and_regenerates() {
    let Some(path) = sections() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let (first, mut runner, mut cache, _dir) = run(&request);
    cache
        .put(request.cache_key(), b"not a waveform summary")
        .unwrap();
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&asset, &clip, window(0, 3), 8, &mut cache).unwrap(),
        WaveformState::Failed(engine::RuntimeError::CacheCorrupt)
    ));
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(
        wait(&mut runner, job.id),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|snapshot| snapshot.error)
    );
    let again = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(again, first);
    assert!(matches!(
        display(&asset, &clip, window(0, 3), 8, &mut cache).unwrap(),
        WaveformState::Ready(_)
    ));
}

#[test]
fn project_json_stays_byte_identical_after_generation() {
    let Some(path) = sections() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let mut data = Project::empty(ProjectId(1), "Waveform".into())
        .unwrap()
        .into_data();
    data.assets.push(asset.clone());
    data.scene_mut().timeline.audio.push(Track {
        id: TrackId(1),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![item(&asset, range(0, 3), Tick::ZERO, 1.0)],
    });
    let project = Project::new(data).unwrap();
    let before = project::encode(&project, false).unwrap();
    let (_summary, _runner, _cache, _dir) = run(&WaveformRequest::new(asset).unwrap());
    let after = project::encode(&project, false).unwrap();
    assert_eq!(before, after);
}

#[test]
fn outside_asset_and_hostile_columns_are_typed_refusals() {
    let Some(path) = sections() else {
        return;
    };
    let asset = source_asset(&path, 1);
    let other = source_asset(&path, 2);
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let (_summary, _runner, mut cache, _dir) = run(&request);
    let mismatched = item(&other, range(0, 3), Tick::ZERO, 1.0);
    let visible = window(0, 3);
    assert!(matches!(
        display(&asset, &mismatched, visible, 8, &mut cache),
        Err(AnalysisError::Edit(engine::EditError::Invalid(
            "waveform display asset"
        )))
    ));
    let clip = item(&asset, range(0, 3), Tick::ZERO, 1.0);
    assert!(matches!(
        display(&asset, &clip, window(0, 1), MAX_COLUMNS + 1, &mut cache),
        Err(AnalysisError::Edit(engine::EditError::Invalid(
            "waveform display columns"
        )))
    ));
    assert!(matches!(
        display(&asset, &clip, window(4, 5), 8, &mut cache),
        Err(AnalysisError::Edit(engine::EditError::Invalid(
            "waveform display outside clip"
        )))
    ));
}

/// Manual check against a real 30-minute interview. Not part of CI: the sample
/// is local and may be absent. Reports generation time; peak RSS is measured by
/// the caller around this process.
#[test]
#[ignore = "manual: set VIBECUT_INTERVIEW_SAMPLE to interview-30m.mp4"]
fn interview_generates_a_complete_summary() {
    let Some(path) = std::env::var_os("VIBECUT_INTERVIEW_SAMPLE").map(PathBuf::from) else {
        eprintln!("skipping: VIBECUT_INTERVIEW_SAMPLE is not set");
        return;
    };
    if !path.is_file() {
        eprintln!("skipping: {} does not exist", path.display());
        return;
    }
    let asset = SourceAsset::import(AssetId(30), path).unwrap();
    let request = WaveformRequest::new(asset.clone()).unwrap();
    let started = Instant::now();
    let (summary, _runner, mut cache, _dir) = run(&request);
    let generated = started.elapsed();
    let bytes = summary.encode().unwrap();
    assert!(summary.channels() == 1 || summary.channels() == 2);
    assert!(summary.total_samples() > 1_400 * 48_000);
    let duration = asset.probe().duration.unwrap();
    let clip = item(
        &asset,
        SourceRange::new(Tick::ZERO, duration).unwrap(),
        Tick::ZERO,
        1.0,
    );
    let visible = TimeRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND * 60)).unwrap();
    assert!(matches!(
        display(&asset, &clip, visible, 800, &mut cache).unwrap(),
        WaveformState::Ready(_)
    ));
    println!(
        "waveform interview: rate={} channels={} samples={} buckets={} summary_bytes={} generated={generated:?}",
        summary.sample_rate(),
        summary.channels(),
        summary.total_samples(),
        summary.bucket_count(),
        bytes.len()
    );
}

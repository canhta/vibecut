#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and assertions"
)]
use engine::{
    ByteBudget, EditError, Session,
    cache::ArtifactCache,
    jobs::{JobRunner, JobState, RunnerConfig, SystemClock},
    scene_detection::*,
};
use media::{AssetId, SourceAsset};
use project::{Item, ItemId, ItemParams, Project, ProjectId, VideoParams};
use std::{
    io::Write,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
fn frame(n: i64) -> Tick {
    Tick(n * (TICKS_PER_SECOND / 10))
}
fn range(n: i64) -> SourceRange {
    SourceRange::new(Tick::ZERO, frame(n)).unwrap()
}
/// Fixture ownership is per process: another CI job must never truncate a
/// source this process is still decoding.
pub(super) fn fixture(name: &str, size: u32, count: usize, cuts: bool) -> SourceAsset {
    let dir = testkit::fixture::dir("scene-detection");
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
            .arg(format!("{size}x{size}"))
            .args([
                "-framerate",
                "10",
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
        for n in 0..count {
            let color = if name == "slice-cut" {
                if n < 3000 { [0, 0, 0] } else { [255, 255, 255] }
            } else if name.starts_with("marker-overflow") {
                if n.is_multiple_of(2) {
                    [0, 0, 0]
                } else {
                    [255, 255, 255]
                }
            } else if !cuts || n < 10 {
                [0, 0, 0]
            } else if n < 20 {
                [255, 255, 255]
            } else if n <= 30 {
                [255, 0, 0]
            } else if n <= 40 {
                let blue = ((n - 30) * 255 / 10) as u8;
                [255 - blue, 0, blue]
            } else {
                [0, 0, 255]
            };
            let mut pixels = vec![0; (size * size * 3) as usize];
            for pixel in pixels.as_chunks_mut::<3>().0 {
                pixel.copy_from_slice(&color);
            }
            if name == "same-histogram" {
                for y in 0..size as usize {
                    for x in 0..size as usize {
                        let value = if (x < size as usize / 2) == (n < 10) {
                            0
                        } else {
                            255
                        };
                        pixels[(y * size as usize + x) * 3..(y * size as usize + x) * 3 + 3]
                            .fill(value);
                    }
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
    setup_clock(Arc::new(SystemClock::default()))
}
fn setup_clock(
    clock: Arc<dyn engine::jobs::Clock>,
) -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    let budget = ByteBudget::new(256 << 20).unwrap();
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
        clock,
    )
    .unwrap();
    (runner, cache, dir)
}
fn wait(runner: &mut JobRunner, job: &DetectionJob) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(120);
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
fn detect(request: &DetectionRequest) -> DetectionResult {
    let (mut runner, mut cache, _dir) = setup();
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(
        wait(&mut runner, &job),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).and_then(|s| s.error)
    );
    job.finish(&mut runner, &mut cache).unwrap().unwrap()
}
fn session(asset: SourceAsset, count: i64) -> Session {
    let mut data = Project::empty(ProjectId(1), "Scenes".into())
        .unwrap()
        .into_data();
    data.assets.push(asset);
    data.scene_mut().timeline.main.items.push(Item {
        hidden: false,
        id: ItemId(10),
        source_asset: Some(AssetId(1)),
        source_range: range(count),
        start: Tick::ZERO,
        retime: Default::default(),
        blend_mode: Default::default(),
        masks: vec![],
        effects: vec![],
        animations: Default::default(),
        content: ItemParams::Video(VideoParams {
            source_audio_enabled: false,
            volume: 0.,
            muted: false,
            fades: Default::default(),
        }),
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
#[test]
fn known_cuts_fade_and_two_uncached_runs_match_exact_frames() {
    let asset = fixture("cuts-fade", 64, 50, true);
    assert_eq!(
        asset.path().parent().unwrap().file_name().unwrap(),
        std::process::id().to_string().as_str()
    );
    let request = DetectionRequest::new(asset, range(50), Default::default()).unwrap();
    let a = detect(&request);
    let b = detect(&request);
    assert_eq!(a, b);
    assert_eq!(a.decoded_frames, 50);
    assert_eq!(
        a.candidates
            .iter()
            .map(|c| (c.at, c.kind))
            .collect::<Vec<_>>(),
        vec![
            (frame(10), ChangeKind::HardCut),
            (frame(20), ChangeKind::HardCut),
            (frame(35), ChangeKind::Fade)
        ]
    );
    assert!(
        a.candidates
            .iter()
            .all(|c| c.confidence > 0. && c.confidence <= 1.)
    );
}
#[test]
fn reviewed_subset_is_one_undo_step_and_refusals_are_atomic() {
    let asset = fixture("review", 64, 50, true);
    let result =
        detect(&DetectionRequest::new(asset.clone(), range(50), Default::default()).unwrap());
    let mut s = session(asset, 50);
    let original = serde_json::to_vec(s.project().data()).unwrap();
    for selection in [&[0, 0][..], &[0, 99][..]] {
        assert!(apply_reviewed(&mut s, ItemId(10), &result, selection).is_err());
        assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), original);
        assert_eq!(s.history_len(), (0, 0));
    }
    apply_reviewed(&mut s, ItemId(10), &result, &[]).unwrap();
    assert_eq!(s.history_len(), (0, 0));
    apply_reviewed(&mut s, ItemId(10), &result, &[2, 0]).unwrap();
    let applied = serde_json::to_vec(s.project().data()).unwrap();
    let items = &s.project().data().scene().timeline.main.items;
    assert_eq!(items.len(), 3);
    assert_eq!(
        items.iter().map(|i| i.start).collect::<Vec<_>>(),
        vec![Tick::ZERO, frame(10), frame(35)]
    );
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), original);
    s.redo().unwrap();
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), applied);
    s.undo().unwrap();
    s.execute(
        "timeline.set_speed",
        serde_json::json!({"items":[10],"speed":2.0,"maintain_pitch":true}),
    )
    .unwrap();
    let retimed = serde_json::to_vec(s.project().data()).unwrap();
    apply_reviewed(&mut s, ItemId(10), &result, &[0, 2]).unwrap();
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .iter()
            .map(|i| i.start)
            .collect::<Vec<_>>(),
        vec![Tick::ZERO, frame(5), Tick(frame(35).0 / 2)]
    );
    s.undo().unwrap();
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), retimed);
    // A reviewed subset larger than one edit batch still undoes in one step.
    let asset = fixture("marker-overflow-review", 16, 1026, true);
    let request = DetectionRequest::new(
        asset.clone(),
        range(1026),
        DetectionSettings {
            min_scene: Tick::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    let result = detect(&request);
    assert_eq!(result.candidates.len(), 1025);
    let mut s = session(asset, 1026);
    let original = serde_json::to_vec(s.project().data()).unwrap();
    let selected: Vec<_> = (0..result.candidates.len()).collect();
    apply_reviewed(&mut s, ItemId(10), &result, &selected).unwrap();
    assert_eq!(s.project().data().scene().timeline.main.items.len(), 1026);
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), original);
    s.redo().unwrap();
    assert_eq!(s.project().data().scene().timeline.main.items.len(), 1026);
}
#[test]
fn cancellation_preserves_timeline_and_cache_hits_do_not_decode() {
    let asset = fixture("cache", 64, 50, true);
    let request = DetectionRequest::new(asset.clone(), range(50), Default::default()).unwrap();
    let s = session(asset.clone(), 50);
    let original = serde_json::to_vec(s.project().data()).unwrap();
    let (mut runner, mut cache, _dir) = setup();
    let cancelled = request.submit(&mut runner, &mut cache).unwrap();
    cancelled.cancel.cancel();
    assert_eq!(wait(&mut runner, &cancelled), JobState::Cancelled);
    assert!(matches!(
        cancelled.finish(&mut runner, &mut cache),
        Err(EditError::Cancelled)
    ));
    assert!(cache.get(request.cache_key()).unwrap().is_none());
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), original);
    assert_eq!(s.history_len(), (0, 0));
    let first = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, &first), JobState::Completed);
    let result = first.finish(&mut runner, &mut cache).unwrap().unwrap();
    let path = asset.path().to_path_buf();
    let hidden = path.with_extension("hidden");
    std::fs::rename(&path, &hidden).unwrap();
    let second = request.submit(&mut runner, &mut cache).unwrap();
    let state = wait(&mut runner, &second);
    std::fs::rename(hidden, path).unwrap();
    assert_eq!(state, JobState::Completed);
    assert_eq!(
        second.finish(&mut runner, &mut cache).unwrap().unwrap(),
        result
    );
    let settings = DetectionSettings::default();
    let changes = [
        DetectionSettings {
            hard_threshold: 0.5,
            ..settings
        },
        DetectionSettings {
            fade_threshold: 0.3,
            ..settings
        },
        DetectionSettings {
            fade_step: 0.01,
            ..settings
        },
        DetectionSettings {
            min_scene: frame(1),
            ..settings
        },
    ];
    let mut keys = std::collections::BTreeSet::from([request.cache_key()]);
    for settings in changes {
        let changed = DetectionRequest::new(asset.clone(), range(50), settings).unwrap();
        assert!(keys.insert(changed.cache_key()));
    }
    let cropped = DetectionRequest::new(
        asset,
        SourceRange::new(frame(1), frame(49)).unwrap(),
        settings,
    )
    .unwrap();
    assert!(keys.insert(cropped.cache_key()));
}
#[test]
fn degenerate_media_and_duration_dimension_scaling_stay_bounded() {
    for (size, count) in [(16, 1), (16, 10), (16, 3100), (512, 10)] {
        let asset = fixture(&format!("constant-{size}-{count}"), size, count, false);
        let result =
            detect(&DetectionRequest::new(asset, range(count as i64), Default::default()).unwrap());
        assert!(result.candidates.is_empty());
        assert_eq!(result.decoded_frames, count as u64);
        eprintln!("scene scaling {size}x{size} frames={count}: exact frame count, no candidates");
    }
    let asset = fixture("hostile", 16, 1, false);
    assert!(SourceRange::new(Tick::ZERO, Tick::ZERO).is_err());
    let mut admitted: media::AssetData = asset.clone().into();
    admitted.probe.streams[0].video.as_mut().unwrap().width = 8192;
    admitted.probe.streams[0].video.as_mut().unwrap().height = 4096;
    assert!(
        DetectionRequest::new(
            SourceAsset::try_from(admitted).unwrap(),
            range(1),
            Default::default()
        )
        .is_err()
    );
    let mut data: media::AssetData = asset.clone().into();
    data.probe.streams[0].video.as_mut().unwrap().width = 65536;
    data.probe.streams[0].video.as_mut().unwrap().height = 65536;
    let huge = SourceAsset::try_from(data);
    // Media admission can reject this geometry before the detector sees it.
    if let Ok(asset) = huge {
        assert!(DetectionRequest::new(asset, range(1), Default::default()).is_err());
    }
    for threshold in [f32::NAN, f32::INFINITY, 0., 1.01] {
        assert!(
            DetectionRequest::new(
                asset.clone(),
                range(1),
                DetectionSettings {
                    hard_threshold: threshold,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    let empty = asset.path().with_extension("empty");
    std::fs::write(&empty, []).unwrap();
    assert!(SourceAsset::import(AssetId(2), empty).is_err());
}

// Clock is the runner's existing injectable boundary. Its first worker call is
// frame progress, after a real decoded frame; gates avoid timing-based polling.
struct FrameGate {
    arrived: std::sync::mpsc::SyncSender<()>,
    resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    first: std::sync::atomic::AtomicBool,
}
impl engine::jobs::Clock for FrameGate {
    fn now(&self) -> Duration {
        if std::thread::current().name() == Some("vibecut-job")
            && !self.first.swap(true, std::sync::atomic::Ordering::SeqCst)
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
fn cancellation_after_decoded_frame_leaves_no_cache_or_timeline_edit() {
    let asset = fixture("running-cancel", 64, 50, true);
    let request = DetectionRequest::new(asset.clone(), range(50), Default::default()).unwrap();
    let s = session(asset, 50);
    let before = serde_json::to_vec(s.project().data()).unwrap();
    let (arrived_tx, arrived_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let clock = Arc::new(FrameGate {
        arrived: arrived_tx,
        resume: std::sync::Mutex::new(resume_rx),
        first: std::sync::atomic::AtomicBool::new(false),
    });
    let (mut runner, mut cache, _dir) = setup_clock(clock);
    let job = request.submit(&mut runner, &mut cache).unwrap();
    runner.tick().unwrap();
    arrived_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(runner.snapshot(job.id).unwrap().state, JobState::Running);
    job.cancel.cancel();
    resume_tx.send(()).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Cancelled);
    assert!(matches!(
        job.finish(&mut runner, &mut cache),
        Err(EditError::Cancelled)
    ));
    assert!(cache.get(request.cache_key()).unwrap().is_none());
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), before);
    assert_eq!(s.history_len(), (0, 0));
}

#[test]
fn excessive_changes_fail_instead_of_retaining_unbounded_markers() {
    let asset = fixture("marker-overflow", 16, 4100, true);
    let request = DetectionRequest::new(
        asset,
        range(4100),
        DetectionSettings {
            min_scene: Tick::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    let (mut runner, mut cache, _dir) = setup();
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Failed);
    assert!(job.finish(&mut runner, &mut cache).is_err());
    assert!(cache.get(request.cache_key()).unwrap().is_none());
}

#[test]
fn continuous_mandelbrot_zoom_is_not_a_fade() {
    let dir = testkit::fixture::dir("scene-detection");
    let Some(path) = testkit::oracle::generate(
        &dir,
        "mandelbrot zoom",
        "mandelbrot-zoom.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "mandelbrot=size=128x96:rate=10",
            "-t",
            "12",
            "-c:v",
            "ffv1",
        ],
    ) else {
        return;
    };
    let asset = SourceAsset::import(AssetId(1), path).unwrap();
    let result = detect(&DetectionRequest::new(asset, range(120), Default::default()).unwrap());
    assert!(
        !result.candidates.iter().any(|c| c.kind == ChangeKind::Fade),
        "{:?}",
        result.candidates
    );
}

#[test]
fn cut_at_decode_slice_boundary_is_neither_lost_nor_duplicated() {
    let asset = fixture("slice-cut", 16, 3100, true);
    let result = detect(&DetectionRequest::new(asset, range(3100), Default::default()).unwrap());
    assert_eq!(result.decoded_frames, 3100);
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|c| (c.at, c.kind))
            .collect::<Vec<_>>(),
        vec![(frame(3000), ChangeKind::HardCut)]
    );
}

/// Fresh subprocesses measure the real decode/job path, without fixture generation
/// or other test threads in the RSS sample. This is a Mac host acceptance test.
#[cfg(target_os = "macos")]
#[test]
fn duration_scaling_bounds_peak_memory() {
    const PROBE: &str = "VIBECUT_SCENE_MEMORY_PROBE";
    if let Some(path) = std::env::var_os(PROBE) {
        let asset = SourceAsset::import(AssetId(1), std::path::PathBuf::from(path)).unwrap();
        let source = SourceRange::new(Tick::ZERO, asset.probe().duration.unwrap()).unwrap();
        let result = detect(&DetectionRequest::new(asset, source, Default::default()).unwrap());
        assert_eq!(
            result.decoded_frames,
            (source.duration().0 / frame(1).0) as u64
        );
        assert!(result.candidates.is_empty());
        return;
    }
    let mut peaks = Vec::new();
    for count in [300, 12000] {
        let asset = fixture(&format!("memory-{count}"), 64, count, false);
        let output = Command::new("/usr/bin/time")
            .arg("-l")
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "analysis::scene_detection::duration_scaling_bounds_peak_memory",
                "--nocapture",
            ])
            .env(PROBE, asset.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "RSS probe did not run"
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        let peak = stderr
            .lines()
            .find(|line| line.contains("maximum resident set size"))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        peaks.push(peak);
        eprintln!("scene memory: frames={count} peak_rss_bytes={peak}");
        assert!(peak < 192 << 20, "unbounded resident memory: {peak}");
    }
    assert!(
        peaks[1] <= peaks[0] + (16 << 20),
        "duration-dependent retained memory: {peaks:?}"
    );
}

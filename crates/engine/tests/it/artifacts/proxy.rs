#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and explicit assertions"
)]
use engine::{
    ByteBudget,
    cache::CacheOperation,
    control::MediaAccess,
    jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock},
    proxies::{ProxyFallback, ProxyIndex, ProxyReady, ProxySources},
};
use media::{AssetId, SourceAsset};
use project::ItemId;
use render::{FileSources, Image, SourceProvider};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use time::{TICKS_PER_SECOND, Tick, TimeRange};

/// Recorded bound: mean absolute difference per channel of the renderer's
/// working pixels (0..1) between a source frame and its proxy frame at the same
/// time. An engineering bound set before the first measurement.
const PREVIEW_TOLERANCE: f32 = 0.06;

struct Hooks;
struct Lease;
impl render::Reservation for Lease {}
impl render::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(Lease))
    }
}

fn fixture(name: &str, filter: &str) -> Option<PathBuf> {
    if !testkit::oracle::require("preview proxy") {
        return None;
    }
    testkit::oracle::generate(
        &testkit::fixture::dir("proxy"),
        "preview proxy",
        name,
        &[
            "-f", "lavfi", "-i", filter, "-c:v", "ffv1", "-pix_fmt", "yuv420p",
        ],
    )
}

/// 3 s of 10 fps video with a moving pattern.
fn source() -> Option<PathBuf> {
    fixture("source.mkv", "testsrc2=s=128x72:r=10:d=3")
}

/// Same raster and rate, but solid red: a proxy that is plainly not the source.
fn poisoned() -> Option<PathBuf> {
    fixture("poisoned.mkv", "color=c=red:s=128x72:r=10:d=3")
}

fn setup() -> (tempfile::TempDir, ByteBudget, MediaAccess, JobRunner) {
    let dir = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(768 << 20).unwrap();
    let access = MediaAccess::open_in(dir.path(), budget.clone()).unwrap();
    let runner = JobRunner::new(
        RunnerConfig {
            concurrency: [2; 3],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    (dir, budget, access, runner)
}

fn wait(runner: &mut JobRunner, id: JobId) -> JobState {
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
        assert!(Instant::now() < deadline, "proxy generation timeout");
        std::thread::yield_now();
    }
}

fn generate(access: &mut MediaAccess, runner: &mut JobRunner, asset: &SourceAsset) -> ProxyReady {
    let id = access.generate_proxy(runner, asset.clone()).unwrap();
    assert_eq!(
        wait(runner, id),
        JobState::Completed,
        "{:?}",
        runner.snapshot(id).and_then(|snapshot| snapshot.error)
    );
    access.finish_proxy(runner, id).unwrap().unwrap()
}

fn clear(access: &MediaAccess) -> u64 {
    access
        .cache_maintenance()
        .unwrap()
        .into_iter()
        .map(|snapshot| {
            let report = snapshot.run(CacheOperation::Clear, || false);
            assert!(report.cause.is_none(), "{:?}", report.cause);
            report.removed_bytes
        })
        .sum()
}

fn disk_bytes(access: &MediaAccess) -> u64 {
    access
        .cache_maintenance()
        .unwrap()
        .into_iter()
        .map(|snapshot| snapshot.run(CacheOperation::Inspect, || false).disk_bytes)
        .sum()
}

fn frame(provider: &mut dyn SourceProvider, asset: &SourceAsset, at: Tick) -> Image {
    provider
        .video(asset, ItemId(1), at, &Hooks)
        .unwrap()
        .unwrap()
}

fn mean_difference(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let total: f32 = a
        .pixels()
        .iter()
        .zip(b.pixels())
        .map(|(a, b)| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f32>())
        .sum();
    total / (a.pixels().len() * 3) as f32
}

fn mean_color(image: &Image) -> [f32; 3] {
    let mut sum = [0.0; 3];
    for pixel in image.pixels() {
        for (total, channel) in sum.iter_mut().zip(pixel) {
            *total += channel;
        }
    }
    sum.map(|total| total / image.pixels().len() as f32)
}

fn video_project(asset: &SourceAsset) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(1), "p".into())
        .unwrap()
        .into_data();
    let shape = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())
        .unwrap();
    data.canvas.width = shape.width;
    data.canvas.height = shape.height;
    data.canvas.fps = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
        .unwrap();
    let duration = asset.probe().duration.unwrap();
    data.assets.push(asset.clone());
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(1),
            source_asset: Some(asset.id()),
            source_range: TimeRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.0,
                muted: false,
                crop: None,
                cleanup: Default::default(),
                reduce_noise: false,
                background: project::ClipBackground::None,
                fit: Default::default(),
            }),
        }],
    });
    project::Project::new(data).unwrap()
}

fn mean_byte_difference(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / a.len() as f64
}

#[test]
fn preview_matches_the_source_within_tolerance_and_the_toggle_needs_no_reopen() {
    let Some(path) = source() else {
        return;
    };
    let asset = SourceAsset::import(AssetId(1), path).unwrap();
    let (_dir, _budget, mut access, mut runner) = setup();
    assert!(generate(&mut access, &mut runner, &asset).generated);
    let index = ProxyIndex::default();
    assert!(access.adopt_proxy(&asset, &index).unwrap());
    // One provider for the whole test: nothing is reopened to change the choice.
    let mut preview = ProxySources::new(index.clone());
    let mut reference = FileSources::default();
    for seconds in [0.0, 0.5, 1.3, 2.4] {
        let at = Tick((seconds * TICKS_PER_SECOND as f64) as i64);
        let original = frame(&mut reference, &asset, at);
        index.set_enabled(false);
        assert_eq!(
            frame(&mut preview, &asset, at).pixels(),
            original.pixels(),
            "proxies off reads the source"
        );
        index.set_enabled(true);
        let proxied = frame(&mut preview, &asset, at);
        let difference = mean_difference(&proxied, &original);
        assert!(
            difference <= PREVIEW_TOLERANCE,
            "proxy frame at {seconds} s differs by {difference}"
        );
    }
    index.set_enabled(false);
    assert!(!index.enabled());
}

#[test]
fn export_reads_the_source_while_preview_reads_a_poisoned_proxy() {
    let (Some(path), Some(red)) = (source(), poisoned()) else {
        return;
    };
    let asset = SourceAsset::import(AssetId(1), path.clone()).unwrap();
    let index = ProxyIndex::default();
    index
        .adopt(&asset, red.clone(), std::fs::metadata(&red).unwrap().len())
        .unwrap();
    index.set_enabled(true);
    let mut preview = ProxySources::new(index);
    let seen = mean_color(&frame(&mut preview, &asset, Tick::ZERO));
    assert!(
        seen[0] > 0.5 && seen[1] < 0.2 && seen[2] < 0.2,
        "preview resolves to the proxy: {seen:?}"
    );
    // Export uses render's own source provider and never receives the index.
    let project = video_project(&asset);
    let out = tempfile::tempdir().unwrap();
    let exported = out.path().join("export.mp4");
    let mut settings = render::ExportSettings::from_project(&project);
    settings.include_audio = false;
    let range = TimeRange::new(Tick::ZERO, project.duration()).unwrap();
    render::export(&project, range, &settings, &exported, &Hooks).unwrap();
    assert_eq!(
        testkit::oracle::stream_field(&exported, "v:0", "width"),
        "128"
    );
    let first = testkit::oracle::rgba(&exported);
    let from_source = mean_byte_difference(&first, &testkit::oracle::rgba(&path));
    let from_proxy = mean_byte_difference(&first, &testkit::oracle::rgba(&red));
    assert!(
        from_source < from_proxy,
        "export frame is {from_source} from the source and {from_proxy} from the proxy"
    );
}

#[test]
fn clearing_the_cache_while_open_falls_back_to_the_source_and_regenerates() {
    let Some(path) = source() else {
        return;
    };
    let asset = SourceAsset::import(AssetId(1), path).unwrap();
    let (dir, budget, mut access, mut runner) = setup();
    assert!(generate(&mut access, &mut runner, &asset).generated);
    // Disk accounting: the cache report, the budget and the files agree.
    let on_disk: u64 = std::fs::read_dir(dir.path().join("proxies"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".mp4"))
        .map(|entry| entry.metadata().unwrap().len())
        .sum();
    assert!(on_disk > 0);
    assert_eq!(disk_bytes(&access), on_disk);
    assert_eq!(budget.snapshot().unwrap().used, on_disk);
    let index = ProxyIndex::default();
    assert!(access.adopt_proxy(&asset, &index).unwrap());
    index.set_enabled(true);
    let at = Tick(TICKS_PER_SECOND);
    let original = frame(&mut FileSources::default(), &asset, at);
    let held = frame(&mut ProxySources::new(index.clone()), &asset, at);
    assert!(mean_difference(&held, &original) <= PREVIEW_TOLERANCE);
    assert!(index.fallbacks().is_empty());

    // The provider above is gone, as between playback runs, so no handle holds
    // the file when the Settings clear removes it.
    assert_eq!(clear(&access), on_disk);
    assert_eq!(budget.snapshot().unwrap().used, 0);
    let mut preview = ProxySources::new(index.clone());
    assert_eq!(
        frame(&mut preview, &asset, at).pixels(),
        original.pixels(),
        "a cleared proxy degrades to the source"
    );
    assert_eq!(
        index.fallbacks(),
        vec![(asset.content_hash(), ProxyFallback::Missing)]
    );
    assert!(!index.contains(asset.content_hash()));
    assert!(!access.adopt_proxy(&asset, &index).unwrap());

    // Regeneration restores the proxy and clears the diagnostic.
    assert!(generate(&mut access, &mut runner, &asset).generated);
    assert!(access.adopt_proxy(&asset, &index).unwrap());
    assert!(index.fallbacks().is_empty());
    assert!(mean_difference(&frame(&mut preview, &asset, at), &original) <= PREVIEW_TOLERANCE);

    // A cache file deleted behind the cache's back also falls back, and the
    // next lookup reports it as corrupt so the entry and its charge retire.
    let file = std::fs::read_dir(dir.path().join("proxies"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "mp4"))
        .unwrap();
    drop(preview);
    std::fs::remove_file(file).unwrap();
    let mut preview = ProxySources::new(index.clone());
    assert_eq!(frame(&mut preview, &asset, at).pixels(), original.pixels());
    assert!(matches!(
        access.adopt_proxy(&asset, &index),
        Err(engine::ArtifactError::Runtime(
            engine::RuntimeError::CacheCorrupt
        ))
    ));
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn proxy_generation_dedupes_warm_hits_open_no_source_and_cancel_leaves_no_file() {
    let Some(path) = source() else {
        return;
    };
    let asset = SourceAsset::import(AssetId(1), path.clone()).unwrap();
    let (dir, _budget, mut access, mut runner) = setup();
    let first = access.generate_proxy(&mut runner, asset.clone()).unwrap();
    assert_eq!(
        access.generate_proxy(&mut runner, asset.clone()).unwrap(),
        first,
        "one proxy job per content"
    );
    assert_eq!(wait(&mut runner, first), JobState::Completed);
    assert!(
        access
            .finish_proxy(&mut runner, first)
            .unwrap()
            .unwrap()
            .generated
    );
    let proxies = dir.path().join("proxies");
    let staged = |dir: &Path| {
        std::fs::read_dir(dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".vibecut-stage")
            })
            .count()
    };
    assert_eq!(staged(&proxies), 0);

    // Warm: the source can be gone and the job still completes.
    let moved: &Path = &path.with_extension("moved");
    std::fs::rename(&path, moved).unwrap();
    let warm = access.generate_proxy(&mut runner, asset.clone()).unwrap();
    assert_eq!(wait(&mut runner, warm), JobState::Completed);
    assert!(
        !access
            .finish_proxy(&mut runner, warm)
            .unwrap()
            .unwrap()
            .generated
    );
    std::fs::rename(moved, &path).unwrap();

    // Cancel: nothing is published and no staging file survives.
    assert!(clear(&access) > 0);
    let job = access.generate_proxy(&mut runner, asset.clone()).unwrap();
    access.cancel_proxy(job);
    let state = wait(&mut runner, job);
    assert!(matches!(state, JobState::Cancelled | JobState::Completed));
    let result = access.finish_proxy(&mut runner, job);
    if state == JobState::Cancelled {
        assert!(matches!(
            result,
            Err(engine::ArtifactError::Runtime(
                engine::RuntimeError::Cancelled
            ))
        ));
        assert_eq!(disk_bytes(&access), 0);
    }
    assert_eq!(staged(&proxies), 0);
}

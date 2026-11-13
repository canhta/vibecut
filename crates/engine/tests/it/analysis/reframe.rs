#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unreachable
)]
//! Reframe analysis on generated fixtures. The detector double finds the bright
//! squares the fixture draws, so every expected position is known analytically.
use ai::{
    ExecutionHooks,
    vision::{FaceBox, FaceFrame, SceneFrame, SceneResult, SpeakerChunk, VisionDetector},
};
use engine::{
    AnalysisError, AnalysisSettings, ByteBudget, EditError, ReframeAnalysis, ReframeReason,
    ReframeRequest, ReframeTarget,
    cache::ArtifactCache,
    emit_reframe,
    jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use media::{AssetId, SourceAsset};
use project::{
    AnimationPath, BuiltinPath, Item, ItemId, ItemParams, Param, Project, ProjectId, Segment,
    Track, TrackId, TrackKind,
};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

const WIDTH: f64 = 1280.;
const SEC: i64 = TICKS_PER_SECOND;
/// Output frames of the 25 fps fixture, in ticks.
const FRAME: i64 = TICKS_PER_SECOND / 25;

fn t(n: i64) -> Tick {
    Tick(n * SEC)
}
fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(t(a), t(b)).unwrap()
}
/// Four shots divided by black/grey background cuts: a square sweeping
/// sideways (0-6 s), a big and a small square at rest (6-9 s), no square
/// (9-12 s) and a square that disappears after two seconds (12-16 s).
fn fixture() -> Option<SourceAsset> {
    static FIXTURE: OnceLock<Option<SourceAsset>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            if !testkit::oracle::require("reframe analysis") {
                return None;
            }
            let dir = testkit::fixture::dir("reframe-analysis");
            let graph = "\
color=c=black:s=1280x720:r=25:d=6[b0];color=c=white:s=160x160:r=25:d=6[s0];\
[b0][s0]overlay=x='560+400*sin(2*PI*t/6)':y=280[v0];\
color=c=0x808080:s=1280x720:r=25:d=3[b1];color=c=white:s=140x140:r=25:d=3[big];\
color=c=white:s=80x80:r=25:d=3[small];[b1][big]overlay=x=300:y=290[t1];\
[t1][small]overlay=x=900:y=320[v1];\
color=c=black:s=1280x720:r=25:d=3[v2];\
color=c=0x808080:s=1280x720:r=25:d=4[b3];color=c=white:s=96x96:r=25:d=4[s3];\
[b3][s3]overlay=x=200:y=312:enable='lt(t,2)'[v3];\
[v0][v1][v2][v3]concat=n=4:v=1:a=0[v]";
            let path = testkit::oracle::generate(
                &dir,
                "reframe analysis",
                "four-shots.mkv",
                &["-filter_complex", graph, "-map", "[v]", "-c:v", "ffv1"],
            )?;
            Some(SourceAsset::import(AssetId(1), path).unwrap())
        })
        .clone()
}
fn setup() -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    let budget = ByteBudget::new(3 << 30).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 32).unwrap();
    let runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 8,
            retained: 8,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    (runner, cache, dir)
}
fn wait(r: &mut JobRunner, id: JobId) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        r.tick().unwrap();
        let state = r.snapshot(id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return state;
        }
        assert!(Instant::now() < deadline, "reframe timeout");
        std::thread::yield_now();
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Fail,
    Block,
}
struct Squares {
    calls: Arc<AtomicUsize>,
    mode: Mode,
    started: Option<mpsc::SyncSender<()>>,
}
impl Squares {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            calls,
            mode: Mode::Normal,
            started: None,
        }
    }
}
impl VisionDetector for Squares {
    fn faces(
        &mut self,
        frame: FaceFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<FaceBox>, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Mode::Fail => return Err(ai::Error::Inference),
            Mode::Block => {
                if let Some(started) = self.started.take() {
                    started.send(()).unwrap();
                }
                while !hooks.cancelled() {
                    std::thread::yield_now();
                }
                return Err(ai::Error::Cancelled);
            }
            Mode::Normal => {}
        }
        // The G plane of the 640x384 BGR tensor. Columns holding a pixel brighter
        // than 200 form one blob; blobs are separated by dark columns.
        let plane = &frame.values()[640 * 384..2 * 640 * 384];
        let lit = |x: usize, y: usize| plane[y * 640 + x] > 200.;
        let mut faces = Vec::new();
        let mut x = 0;
        while x < 640 {
            if !(0..384).any(|y| lit(x, y)) {
                x += 1;
                continue;
            }
            let from = x;
            while x < 640 && (0..384).any(|y| lit(x, y)) {
                x += 1;
            }
            let rows: Vec<usize> = (0..384).filter(|y| (from..x).any(|c| lit(c, *y))).collect();
            faces.push(FaceBox {
                landmarks: None,
                x: from as f32,
                y: rows[0] as f32,
                width: (x - from) as f32,
                height: (rows[rows.len() - 1] - rows[0] + 1) as f32,
                confidence: 0.9,
            });
        }
        Ok(faces)
    }
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<f32>, ai::Error> {
        Ok(vec![0.; chunk.frames()])
    }
    fn scene(
        &mut self,
        _: SceneFrame<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<SceneResult, ai::Error> {
        Ok(SceneResult::unknown())
    }
}
fn request(asset: &SourceAsset, models: u8) -> ReframeRequest {
    ReframeRequest::new(
        asset.clone(),
        range(0, 16),
        ReframeTarget::Vertical,
        [models; 32],
        AnalysisSettings::default(),
    )
    .unwrap()
}
fn run(request: &ReframeRequest, detector: Squares) -> (ReframeAnalysis, ArtifactCache) {
    let (mut runner, mut cache, dir) = setup();
    std::mem::forget(dir); // keeps the cache directory for the test's lifetime
    let job = request.submit(&mut runner, &mut cache, detector).unwrap();
    assert!(job.finish(&mut runner, &mut cache).unwrap().is_none());
    let state = wait(&mut runner, job.id);
    assert_eq!(
        state,
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).unwrap().error
    );
    (job.finish(&mut runner, &mut cache).unwrap().unwrap(), cache)
}
/// One analysis shared by the tests that only read it.
fn shared() -> Option<&'static (SourceAsset, ReframeAnalysis)> {
    static SHARED: OnceLock<Option<(SourceAsset, ReframeAnalysis)>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let asset = fixture()?;
            let (analysis, _) = run(
                &request(&asset, 1),
                Squares::new(Arc::new(AtomicUsize::new(0))),
            );
            Some((asset, analysis))
        })
        .as_ref()
}
/// Writes the emitted keys onto the item exactly as the apply step will.
fn applied(analysis: &ReframeAnalysis, mut item: Item) -> (Item, engine::ReframeKeys) {
    let keys = emit_reframe(analysis, &item).unwrap();
    let ItemParams::Video(video) = &mut item.content else {
        unreachable!()
    };
    video.crop = Some(keys.crop);
    for (path, channel) in [
        (BuiltinPath::CropX, &keys.x),
        (BuiltinPath::CropY, &keys.y),
        (BuiltinPath::CropAuto, &keys.auto),
    ] {
        item.animations
            .set(AnimationPath::from(path), Param::Value(channel.clone()))
            .unwrap();
    }
    (item, keys)
}
fn crop_at(item: &Item, local: Tick) -> project::Crop {
    let ItemParams::Video(video) = &item.content else {
        unreachable!()
    };
    video.crop.unwrap().at(&item.animations, local).unwrap()
}
/// Item and asset in a project, so `Project::new` checks every key bound.
fn validated(item: Item, seconds: i64) -> Project {
    let mut data = Project::empty(ProjectId(1), "Reframe".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("vibecut-reframe-source.mkv"))
            .format("matroska")
            .duration(t(seconds))
            .hash([7; 32])
            .stream(
                testkit::source::video()
                    .size(1280, 720)
                    .duration(t(seconds))
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.main = Track {
        id: TrackId(1),
        name: "Main".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![item],
    };
    Project::new(data).unwrap()
}
fn percentile(mut values: Vec<f64>, p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * p).ceil() as usize]
}

#[test]
fn the_moving_subject_stays_in_the_safe_area_without_jitter_and_each_fallback_is_reported() {
    let Some((_, analysis)) = shared() else {
        return;
    };
    assert_eq!(
        analysis.shots.iter().map(|s| s.range).collect::<Vec<_>>(),
        vec![range(0, 6), range(6, 9), range(9, 12), range(12, 16)]
    );
    assert_eq!(
        analysis
            .shots
            .iter()
            .map(|s| s.reasons.clone())
            .collect::<Vec<_>>(),
        vec![
            vec![],
            vec![ReframeReason::Ambiguous],
            vec![ReframeReason::NoSubject],
            vec![ReframeReason::Lost]
        ]
    );
    assert_eq!(analysis.crop_size(), (405. / 1280., 1.));
    // Item over the whole source: evaluate the emitted keys at every output frame.
    let (item, keys) = applied(
        analysis,
        testkit::item::video(ItemId(1))
            .asset(AssetId(1))
            .source(range(0, 16))
            .start(Tick::ZERO)
            .build(),
    );
    validated(item.clone(), 16);
    let crop_w = 405.;
    let (mut inside, mut total) = (0, 0);
    let (mut steps, mut subject_steps) = (Vec::new(), Vec::new());
    let (mut previous, mut previous_subject): (Option<f64>, Option<f64>) = (None, None);
    let mut worst: f64 = 0.;
    for n in 0..150 {
        let seconds = n as f64 / 25.;
        let subject = 640. + 400. * (2. * std::f64::consts::PI * seconds / 6.).sin();
        let centre = (crop_at(&item, Tick(n * FRAME)).x + 405. / 2. / WIDTH) * WIDTH;
        worst = worst.max((subject - centre).abs());
        total += 1;
        // Safe area: the central 80 % of the crop window.
        inside += i32::from((subject - centre).abs() <= 0.4 * crop_w);
        if let Some(p) = previous {
            steps.push(((centre - p) / WIDTH).abs());
        }
        if let Some(p) = previous_subject {
            subject_steps.push(((subject - p) / WIDTH).abs());
        }
        previous = Some(centre);
        previous_subject = Some(subject);
    }
    let ratio = f64::from(inside) / f64::from(total);
    let (jitter, subject_jitter) = (percentile(steps, 0.95), percentile(subject_steps, 0.95));
    eprintln!(
        "reframe measured: safe {inside}/{total}, worst offset {worst:.1} px, \
         p95 step {jitter:.5}, subject p95 step {subject_jitter:.5}"
    );
    assert!(ratio >= 0.95, "safe-area ratio {ratio}");
    assert!(jitter <= 0.012, "p95 adjacent step {jitter}");
    assert!(worst <= 60., "worst offset {worst} px");
    assert!(jitter <= subject_jitter, "smoothing never adds motion");
    // The vertical axis is full height: the window never moves.
    assert!((0..150).all(|n| crop_at(&item, Tick(n * FRAME)).y == 0.));
    // Reasons come through the emitter per shot.
    assert_eq!(
        keys.shots
            .iter()
            .map(|s| s.reasons.clone())
            .collect::<Vec<_>>(),
        analysis
            .shots
            .iter()
            .map(|s| s.reasons.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        keys.shots.iter().map(|s| s.start).collect::<Vec<_>>(),
        vec![Tick::ZERO, t(6), t(9), t(12)]
    );
}

#[test]
fn a_cut_resets_the_path_with_a_step_and_the_followed_subject_decides_each_shot() {
    let Some((_, analysis)) = shared() else {
        return;
    };
    let (item, keys) = applied(
        analysis,
        testkit::item::video(ItemId(1))
            .asset(AssetId(1))
            .source(range(0, 16))
            .start(Tick::ZERO)
            .build(),
    );
    let auto = item
        .animations
        .get(&AnimationPath::from(BuiltinPath::CropAuto))
        .unwrap();
    let project::Param::Value(auto) = auto else {
        panic!("scalar channel");
    };
    assert_eq!(
        auto.keys().iter().map(|k| k.time).collect::<Vec<_>>(),
        vec![Tick::ZERO, t(6), t(9), t(12)]
    );
    assert!(auto.keys().iter().all(|k| k.segment == Segment::Step));
    for cut in [t(6), t(9), t(12)] {
        let before = crop_at(&item, Tick(cut.0 - 1)).x;
        let after = crop_at(&item, cut).x;
        assert!(
            (before - after).abs() > 0.05,
            "cut at {cut:?}: {before} then {after}"
        );
        // The key before the cut steps, so the value just before it is held.
        let last = keys.x.keys().iter().rfind(|k| k.time < cut).unwrap();
        assert_eq!(last.segment, Segment::Step);
        assert_eq!(project::ParamValue::Scalar(before), last.value);
    }
    // Largest face of the ambiguous shot is the 140 px square centred at 370.
    let big = (crop_at(&item, t(7)).x + 405. / 2. / WIDTH) * WIDTH;
    assert!((big - 370.).abs() < 40., "{big}");
    // No subject: centred. Lost: starts on the square, ends centred.
    let centre = (1. - 405. / 1280.) / 2.;
    assert!((crop_at(&item, t(10)).x - centre).abs() < 1e-9);
    assert!(crop_at(&item, t(12) + Tick(FRAME)).x < centre - 0.05);
    assert!((crop_at(&item, Tick(t(16).0 - 1)).x - centre).abs() < 0.01);
}

#[test]
fn two_runs_are_byte_equal_and_a_cache_hit_opens_no_source() {
    let Some((asset, analysis)) = shared() else {
        return;
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let (second, mut cache) = run(&request(asset, 1), Squares::new(calls.clone()));
    assert!(calls.load(Ordering::SeqCst) > 0);
    assert_eq!(
        serde_json::to_vec(&second).unwrap(),
        serde_json::to_vec(analysis).unwrap()
    );
    // The same analysis through a source whose file does not exist: a hit never
    // opens it and never asks the detector.
    let missing =
        testkit::source::asset(2, std::env::temp_dir().join("vibecut-reframe-absent.mkv"))
            .format("matroska")
            .duration(t(16))
            .hash(asset.content_hash().0)
            .stream(
                testkit::source::video()
                    .size(1280, 720)
                    .duration(t(16))
                    .build(),
            )
            .build();
    let hit_request = request(&missing, 1);
    assert_eq!(hit_request.cache_key(), request(asset, 1).cache_key());
    let (mut runner, _, _dir) = setup();
    let before = calls.load(Ordering::SeqCst);
    let job = hit_request
        .submit(&mut runner, &mut cache, Squares::new(calls.clone()))
        .unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let hit = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(&hit, analysis);
    assert_eq!(calls.load(Ordering::SeqCst), before);
    // Model, target and range are part of the key.
    for other in [
        request(asset, 2).cache_key(),
        ReframeRequest::new(
            asset.clone(),
            range(0, 16),
            ReframeTarget::Square,
            [1; 32],
            AnalysisSettings::default(),
        )
        .unwrap()
        .cache_key(),
        ReframeRequest::new(
            asset.clone(),
            range(0, 15),
            ReframeTarget::Vertical,
            [1; 32],
            AnalysisSettings::default(),
        )
        .unwrap()
        .cache_key(),
    ] {
        assert_ne!(other, hit_request.cache_key());
    }
}

#[test]
fn trims_retimes_and_repeated_spans_each_get_their_own_keys_from_one_analysis() {
    let Some((_, analysis)) = shared() else {
        return;
    };
    // A trim of the moving shot: the first key is the path at the trim start.
    let (trimmed, keys) = applied(
        analysis,
        testkit::item::video(ItemId(1))
            .asset(AssetId(1))
            .source(range(2, 4))
            .start(t(100))
            .build(),
    );
    validated(trimmed.clone(), 16);
    let first = analysis.shots[0]
        .samples
        .iter()
        .find(|s| s.at == t(2))
        .unwrap();
    assert!((crop_at(&trimmed, Tick::ZERO).x - first.x).abs() < 1e-12);
    assert_eq!(keys.shots.len(), 1);
    assert!(
        keys.x
            .keys()
            .iter()
            .all(|k| k.time >= Tick::ZERO && k.time < t(2))
    );
    // The same span placed elsewhere carries identical item-relative keys.
    let (again, _) = applied(
        analysis,
        testkit::item::video(ItemId(2))
            .asset(AssetId(1))
            .source(range(2, 4))
            .start(t(7))
            .build(),
    );
    assert_eq!(trimmed.animations, again.animations);
    // Twice as fast: the same source instants land at half the output ticks.
    let mut fast = testkit::item::video(ItemId(3))
        .asset(AssetId(1))
        .source(range(2, 4))
        .start(Tick::ZERO)
        .build();
    fast.retime.speed = time::Speed::from_f64(2.).unwrap();
    let (fast, _) = applied(analysis, fast);
    validated(fast.clone(), 16);
    for source in [t(2) + Tick(SEC / 2), t(3)] {
        let slow_x = crop_at(&trimmed, Tick(source.0 - t(2).0)).x;
        let fast_x = crop_at(&fast, Tick((source.0 - t(2).0) / 2)).x;
        assert!((slow_x - fast_x).abs() < 0.002, "{slow_x} vs {fast_x}");
    }
    // An item straddling a cut gets a step at the cut and both shots' keys.
    let (across, keys) = applied(
        analysis,
        testkit::item::video(ItemId(4))
            .asset(AssetId(1))
            .source(range(5, 7))
            .start(Tick::ZERO)
            .build(),
    );
    validated(across, 16);
    assert_eq!(
        keys.shots.iter().map(|s| s.start).collect::<Vec<_>>(),
        vec![Tick::ZERO, t(1)]
    );
    // Items outside the analysed range or not video are refused.
    assert!(
        emit_reframe(
            analysis,
            &testkit::item::video(ItemId(5))
                .asset(AssetId(1))
                .source(range(0, 17))
                .start(Tick::ZERO)
                .build()
        )
        .is_err()
    );
    let mut audio = testkit::item::video(ItemId(6))
        .asset(AssetId(1))
        .source(range(0, 2))
        .start(Tick::ZERO)
        .build();
    audio.content = ItemParams::Audio(project::AudioParams::default());
    assert!(emit_reframe(analysis, &audio).is_err());
}

/// An analysis whose single shot zigzags by 100 px between samples.
fn zigzag(samples: usize) -> ReframeAnalysis {
    let rows: Vec<serde_json::Value> = (0..samples)
        .map(|i| {
            serde_json::json!({
                "at": i as i64 * (SEC / 8),
                "x": if i % 2 == 0 { 0.1 } else { 0.2 },
                "y": 0.0
            })
        })
        .collect();
    let end = (samples as i64 - 1) * (SEC / 8) + 1;
    serde_json::from_value(serde_json::json!({
        "version": "synthetic",
        "key": vec![0; 32],
        "models": vec![0; 32],
        "range": range_json(0, end),
        "target": "9:16",
        "source": [1280, 720],
        "size": [405. / 1280., 1.0],
        "shots": [{
            "range": range_json(0, end),
            "reasons": [],
            "samples": rows
        }]
    }))
    .unwrap()
}
fn range_json(a: i64, b: i64) -> serde_json::Value {
    serde_json::to_value(SourceRange::new(Tick(a), Tick(b)).unwrap()).unwrap()
}

#[test]
fn keys_drop_by_half_a_pixel_and_a_channel_over_the_limit_is_a_typed_failure() {
    let mut item = testkit::item::video(ItemId(1))
        .asset(AssetId(1))
        .source(range(0, 1))
        .start(Tick::ZERO)
        .build();
    // 100 samples shorter than the limit: every zigzag key is needed.
    let small = zigzag(100);
    item.source_range = SourceRange::new(Tick::ZERO, Tick(99 * (SEC / 8) + 1)).unwrap();
    let keys = emit_reframe(&small, &item).unwrap();
    assert_eq!(keys.x.keys().len(), 100);
    assert_eq!(keys.y.keys().len(), 2, "a constant channel keeps both ends");
    // A straight ramp loses its interior keys: one end to the other.
    let ramp: ReframeAnalysis = {
        let mut a = zigzag(100);
        for (i, s) in a.shots[0].samples.iter_mut().enumerate() {
            s.x = 0.1 + i as f64 * 0.001;
        }
        a
    };
    assert_eq!(emit_reframe(&ramp, &item).unwrap().x.keys().len(), 2);
    // More zigzag keys than a channel holds fail instead of truncating.
    let long = zigzag(4100);
    item.source_range = SourceRange::new(Tick::ZERO, Tick(4099 * (SEC / 8) + 1)).unwrap();
    assert!(matches!(
        emit_reframe(&long, &item),
        Err(AnalysisError::Edit(EditError::Control(
            engine::control::ControlError::Limit
        )))
    ));
}

#[test]
fn failed_cancelled_and_hostile_requests_publish_nothing() {
    let Some(asset) = fixture() else {
        return;
    };
    for mode in [Mode::Fail, Mode::Block] {
        let (mut runner, mut cache, _dir) = setup();
        let (started, receive) = mpsc::sync_channel(1);
        let job = request(&asset, 1)
            .submit(
                &mut runner,
                &mut cache,
                Squares {
                    calls: Arc::new(AtomicUsize::new(0)),
                    mode,
                    started: Some(started),
                },
            )
            .unwrap();
        runner.tick().unwrap();
        if mode == Mode::Block {
            receive.recv_timeout(Duration::from_secs(120)).unwrap();
            job.cancel.cancel();
        }
        assert_eq!(
            wait(&mut runner, job.id),
            if mode == Mode::Block {
                JobState::Cancelled
            } else {
                JobState::Failed
            }
        );
        assert!(job.finish(&mut runner, &mut cache).is_err());
        assert!(runner.output_bytes(job.id).is_none());
        assert!(cache.is_empty());
    }
    for (range, settings) in [
        (range(0, 17), AnalysisSettings::default()),
        (
            range(0, 4),
            AnalysisSettings {
                cut_threshold: 0.,
                ..Default::default()
            },
        ),
        (
            range(0, 4),
            AnalysisSettings {
                min_shot: Tick::ZERO,
                ..Default::default()
            },
        ),
    ] {
        assert!(
            ReframeRequest::new(
                asset.clone(),
                range,
                ReframeTarget::Vertical,
                [1; 32],
                settings
            )
            .is_err()
        );
    }
    // A source without video is unsupported.
    let audio_only =
        testkit::source::asset(3, std::env::temp_dir().join("vibecut-reframe-audio.m4a"))
            .duration(t(4))
            .hash([9; 32])
            .stream(testkit::source::audio().duration(t(4)).build())
            .build();
    assert!(matches!(
        ReframeRequest::new(
            audio_only,
            range(0, 4),
            ReframeTarget::Vertical,
            [1; 32],
            AnalysisSettings::default()
        ),
        Err(AnalysisError::Edit(EditError::Unsupported(_)))
    ));
}

#[test]
fn square_and_landscape_targets_move_the_right_axis() {
    let Some(asset) = fixture() else {
        return;
    };
    for (target, expected) in [
        (ReframeTarget::Square, (720. / 1280., 1.)),
        (ReframeTarget::Landscape, (1., 1.)),
    ] {
        let request = ReframeRequest::new(
            asset.clone(),
            range(0, 2),
            target,
            [1; 32],
            AnalysisSettings::default(),
        )
        .unwrap();
        let (analysis, _) = run(&request, Squares::new(Arc::new(AtomicUsize::new(0))));
        assert_eq!(analysis.crop_size(), expected);
        let (item, keys) = applied(
            &analysis,
            testkit::item::video(ItemId(1))
                .asset(AssetId(1))
                .source(range(0, 2))
                .start(Tick::ZERO)
                .build(),
        );
        validated(item, 16);
        assert!(keys.crop.valid());
        if target == ReframeTarget::Landscape {
            // A 16:9 window in a 16:9 frame has nowhere to go.
            assert!(
                keys.x
                    .keys()
                    .iter()
                    .all(|k| k.value == project::ParamValue::Scalar(0.))
            );
        }
    }
}

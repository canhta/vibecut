#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "synthetic fixtures and explicit assertions"
)]
use engine::{
    ByteBudget, EditError,
    cache::ArtifactCache,
    jobs::{Clock, JobRunner, JobState, RunnerConfig, SystemClock},
    speech_activity::{
        Activity, SpeechActivityRequest, SpeechActivitySettings, SpeechActivitySpan,
    },
};
use media::{AssetId, SourceAsset};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

fn ms(value: i64) -> Tick {
    Tick(value * (TICKS_PER_SECOND / 1000))
}
fn t(seconds: i64) -> Tick {
    Tick(seconds * TICKS_PER_SECOND)
}
fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(t(start), t(end)).unwrap()
}

/// 10 s of speech-like tone with three declared silences, a 0.2 s dip that
/// hangover must keep as speech, and a 0.06 s click inside the first pause.
fn fixture() -> Option<SourceAsset> {
    if !testkit::oracle::require("speech activity") {
        return None;
    }
    let dir = testkit::fixture::dir("speech-activity");
    let expression = "(0.25*sin(2*PI*180*t)+0.15*sin(2*PI*320*t)+0.1*sin(2*PI*520*t))*\
        (between(t\\,0\\,1.2)+between(t\\,1.9\\,2.3)+between(t\\,2.5\\,3)+\
         between(t\\,4.1\\,5.2)+between(t\\,6.8\\,10))+\
        0.2*sin(2*PI*330*t)*between(t\\,1.5\\,1.56)";
    let input = format!("aevalsrc={expression}:s=16000:d=10");
    let path = testkit::oracle::generate(
        &dir,
        "speech activity",
        "known-silences.wav",
        &["-f", "lavfi", "-i", input.as_str(), "-c:a", "pcm_s16le"],
    )?;
    Some(SourceAsset::import(AssetId(1), path).unwrap())
}
fn video_only(dir: &Path) -> SourceAsset {
    let path = testkit::oracle::generate(
        dir,
        "speech activity",
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
    SourceAsset::import(AssetId(2), path).unwrap()
}
fn setup() -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    setup_clock(Arc::new(SystemClock::default()))
}
fn setup_clock(clock: Arc<dyn Clock>) -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    let budget = ByteBudget::new(768 << 20).unwrap();
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
        assert!(Instant::now() < deadline, "speech activity timeout");
        std::thread::yield_now();
    }
}
fn run(
    request: &SpeechActivityRequest,
) -> (
    engine::speech_activity::SpeechActivityResult,
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
    let result = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    (result, runner, cache, dir)
}
fn pauses(spans: &[SpeechActivitySpan]) -> Vec<&SpeechActivitySpan> {
    spans
        .iter()
        .filter(|span| span.activity == Activity::Pause)
        .collect()
}

#[test]
fn declared_silences_and_the_continuous_segment_match_within_one_frame() {
    let Some(asset) = fixture() else {
        return;
    };
    let request =
        SpeechActivityRequest::new(asset, range(0, 10), SpeechActivitySettings::default()).unwrap();
    let (result, _runner, _cache, _dir) = run(&request);
    assert_eq!(result.analyzed_frames, 500);
    let pauses = pauses(&result.spans);
    assert_eq!(pauses.len(), 3, "{:?}", result.spans);
    for (span, (start, end)) in pauses
        .iter()
        .zip([(1200, 1900), (3000, 4100), (5200, 6800)])
    {
        let offset = (span.range.start().0 - ms(start).0).abs();
        let trailing = (span.range.end().0 - ms(end).0).abs();
        assert!(
            offset <= ms(20).0,
            "pause start {} vs {start} ms",
            span.range.start().0
        );
        assert!(
            trailing <= ms(20).0,
            "pause end {} vs {end} ms",
            span.range.end().0
        );
    }
    // The 0.2 s dip inside speech is hangover, not a pause; the 0.06 s click
    // inside the first pause is removed without splitting that pause. The open
    // 6.8..10 s segment has no pause.
    assert!(pauses.iter().all(|span| span.range.end() <= ms(6800)));
    assert_eq!(
        result
            .spans
            .iter()
            .filter(|span| span.activity == Activity::Speech)
            .count(),
        4,
        "{:?}",
        result.spans
    );
    let mut end = Tick::ZERO;
    for span in &result.spans {
        assert_eq!(span.range.start(), end);
        assert!(span.range.end() > end);
        assert!((0.0..=1.0).contains(&span.confidence));
        end = span.range.end();
    }
    assert_eq!(end, t(10));
}

#[test]
fn warm_cache_reuses_the_spans_without_opening_the_source() {
    let Some(asset) = fixture() else {
        return;
    };
    let request = SpeechActivityRequest::new(
        asset.clone(),
        range(0, 10),
        SpeechActivitySettings::default(),
    )
    .unwrap();
    let (first, mut runner, mut cache, _dir) = run(&request);
    assert_eq!(cache.len(), 1);
    let path = asset.path().to_path_buf();
    let hidden = path.with_extension("hidden");
    std::fs::rename(&path, &hidden).unwrap();
    let second = request.submit(&mut runner, &mut cache).unwrap();
    let state = wait(&mut runner, second.id);
    std::fs::rename(hidden, path).unwrap();
    assert_eq!(state, JobState::Completed);
    assert_eq!(
        second.finish(&mut runner, &mut cache).unwrap().unwrap(),
        first
    );
    assert_eq!(cache.len(), 1);

    let mut keys = std::collections::BTreeSet::from([request.cache_key()]);
    for settings in [
        SpeechActivitySettings {
            noise_db: -50.,
            ..Default::default()
        },
        SpeechActivitySettings {
            min_pause: ms(600),
            ..Default::default()
        },
        SpeechActivitySettings {
            min_speech: ms(200),
            ..Default::default()
        },
    ] {
        let changed = SpeechActivityRequest::new(asset.clone(), range(0, 10), settings).unwrap();
        assert!(keys.insert(changed.cache_key()));
    }
    let cropped =
        SpeechActivityRequest::new(asset, range(1, 9), SpeechActivitySettings::default()).unwrap();
    assert!(keys.insert(cropped.cache_key()));

    let mut corrupt: serde_json::Value =
        serde_json::from_slice(cache.get(request.cache_key()).unwrap().unwrap().bytes()).unwrap();
    corrupt["spans"][0]["confidence"] = serde_json::json!(2.0);
    cache
        .put(request.cache_key(), &serde_json::to_vec(&corrupt).unwrap())
        .unwrap();
    assert!(matches!(
        request.submit(&mut runner, &mut cache),
        Err(EditError::CacheCorrupt)
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
fn cancelling_after_a_decoded_window_publishes_nothing() {
    let Some(asset) = fixture() else {
        return;
    };
    let request =
        SpeechActivityRequest::new(asset, range(0, 10), SpeechActivitySettings::default()).unwrap();
    let (arrived, arrived_rx) = mpsc::sync_channel(1);
    let (resume, resume_rx) = mpsc::sync_channel(1);
    let clock = Arc::new(FrameGate {
        arrived,
        resume: Mutex::new(resume_rx),
        first: AtomicBool::new(false),
    });
    let (mut runner, mut cache, _dir) = setup_clock(clock);
    let job = request.submit(&mut runner, &mut cache).unwrap();
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
    assert!(cache.is_empty());

    // A request cancelled before the worker starts never opens the source.
    let pre = request.submit(&mut runner, &mut cache).unwrap();
    pre.cancel.cancel();
    assert_eq!(wait(&mut runner, pre.id), JobState::Cancelled);
    assert!(pre.finish(&mut runner, &mut cache).is_err());
    assert!(cache.is_empty());
}

#[test]
fn audio_less_unreadable_and_hostile_inputs_are_typed_failures() {
    let Some(asset) = fixture() else {
        return;
    };
    let dir = asset.path().parent().unwrap().to_path_buf();
    let video = video_only(&dir);
    assert!(matches!(
        SpeechActivityRequest::new(video, range(0, 1), SpeechActivitySettings::default()),
        Err(EditError::Unsupported(_))
    ));

    let copy = asset.path().with_file_name("unreadable.wav");
    std::fs::copy(asset.path(), &copy).unwrap();
    let unreadable = SourceAsset::import(AssetId(3), copy.clone()).unwrap();
    let request =
        SpeechActivityRequest::new(unreadable, range(0, 10), SpeechActivitySettings::default())
            .unwrap();
    std::fs::remove_file(&copy).unwrap();
    let (mut runner, mut cache, _dir) = setup();
    let job = request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Failed);
    assert!(matches!(
        job.finish(&mut runner, &mut cache),
        Err(EditError::JobFailure(inner)) if matches!(inner.as_ref(), EditError::Media(_))
    ));
    assert!(cache.is_empty());

    for settings in [
        SpeechActivitySettings {
            noise_db: f32::NAN,
            ..Default::default()
        },
        SpeechActivitySettings {
            noise_db: 1.0,
            ..Default::default()
        },
        SpeechActivitySettings {
            noise_db: -121.0,
            ..Default::default()
        },
        SpeechActivitySettings {
            min_pause: Tick(-1),
            ..Default::default()
        },
        SpeechActivitySettings {
            min_speech: Tick(-1),
            ..Default::default()
        },
    ] {
        assert!(
            SpeechActivityRequest::new(asset.clone(), range(0, 10), settings).is_err(),
            "hostile settings {settings:?}"
        );
    }
    assert!(
        SpeechActivityRequest::new(
            asset.clone(),
            range(0, 11),
            SpeechActivitySettings::default()
        )
        .is_err()
    );
    assert!(
        SpeechActivityRequest::new(
            asset,
            range(0, 10),
            SpeechActivitySettings {
                min_pause: t(11),
                ..Default::default()
            }
        )
        .is_err()
    );
}

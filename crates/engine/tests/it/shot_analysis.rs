#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use ai::{
    ExecutionHooks,
    vision::{
        FaceBox, FaceFrame, SceneFrame, SceneKind, SceneResult, SpeakerChunk, VisionDetector,
    },
};
use engine::{
    ByteBudget, EditError,
    cache::ArtifactCache,
    jobs::{JobRunner, JobState, RunnerConfig, SystemClock},
    shot_analysis::*,
};
use media::{AssetId, SourceAsset};
use std::{
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
fn t(n: i64) -> Tick {
    Tick(n * TICKS_PER_SECOND)
}
fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(t(a), t(b)).unwrap()
}
fn fixture() -> Option<SourceAsset> {
    static FIXTURE: std::sync::OnceLock<Option<SourceAsset>> = std::sync::OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let mut tool = Command::new("ffmpeg");
            tool.env_remove("DYLD_LIBRARY_PATH")
                .env_remove("LD_LIBRARY_PATH");
            if !tool
                .arg("-version")
                .output()
                .is_ok_and(|r| r.status.success())
            {
                assert!(
                    std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
                    "required ffmpeg missing"
                );
                eprintln!("SKIP shot analysis: ffmpeg unavailable");
                return None;
            }
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/fixtures/shot-analysis");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("three-shots.mkv");
            let result = Command::new("ffmpeg")
                .env_remove("DYLD_LIBRARY_PATH")
                .env_remove("LD_LIBRARY_PATH")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=black:s=64x64:r=25:d=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=white:s=64x64:r=25:d=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=red:s=64x64:r=25:d=1",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=16000:duration=5",
                    "-filter_complex",
                    "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
                    "-map",
                    "[v]",
                    "-map",
                    "3:a",
                    "-c:v",
                    "ffv1",
                    "-c:a",
                    "pcm_s16le",
                    "-shortest",
                ])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            Some(SourceAsset::import(AssetId(1), path).unwrap())
        })
        .clone()
}
fn setup() -> (JobRunner, ArtifactCache, tempfile::TempDir) {
    setup_budget(ByteBudget::new(3 << 30).unwrap())
}
fn setup_budget(budget: ByteBudget) -> (JobRunner, ArtifactCache, tempfile::TempDir) {
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
fn wait(r: &mut JobRunner, id: engine::jobs::JobId) -> JobState {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        r.tick().unwrap();
        let state = r.snapshot(id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return state;
        }
        assert!(Instant::now() < deadline, "analysis timeout");
        std::thread::yield_now();
    }
}
struct Detector {
    calls: Arc<AtomicUsize>,
    mode: u8,
    started: Option<mpsc::SyncSender<()>>,
}
impl Detector {
    fn normal(calls: Arc<AtomicUsize>) -> Self {
        Self {
            calls,
            mode: 0,
            started: None,
        }
    }
}
impl VisionDetector for Detector {
    fn faces(
        &mut self,
        frame: FaceFrame<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<FaceBox>, ai::Error> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            1 => return Err(ai::Error::Inference),
            2 if frame.values()[0] > 200. => {
                self.started.take().unwrap().send(()).unwrap();
                while !hooks.cancelled() {
                    std::thread::yield_now();
                }
                return Err(ai::Error::Cancelled);
            }
            5 if frame.values()[0] > 200. => return Err(ai::Error::Inference),
            3 => panic!("injected detector panic"),
            4 => {
                return Ok(vec![FaceBox {
                    landmarks: None,
                    x: f32::NAN,
                    y: 0.,
                    width: 2.,
                    height: 2.,
                    confidence: 1.,
                }]);
            }
            7 if !call.is_multiple_of(2) => return Ok(vec![]),
            _ => {}
        }
        // Black speaker, white listener, red screen. Face boxes use letterbox pixels.
        if frame.values()[2 * 640 * 384] > 200. && frame.values()[640 * 384] < 20. {
            Ok(vec![])
        } else {
            let mut faces = vec![FaceBox {
                landmarks: (self.mode == 8).then(|| {
                    [
                        [38.2946, 51.6963],
                        [73.5318, 51.5014],
                        [56.0252, 71.7366],
                        [41.5493, 92.3655],
                        [70.7299, 92.2041],
                    ]
                    .map(|[x, y]| [(x * 0.35 + 5.) * 10., (y * 0.35 + 5.) * 10.])
                }),
                x: 64.,
                y: 64.,
                width: 180.,
                height: 180.,
                confidence: 0.9,
            }];
            if self.mode == 6 && call < 3 {
                faces.push(FaceBox {
                    landmarks: None,
                    x: 230.,
                    y: 64.,
                    width: 110.,
                    height: 180.,
                    confidence: 0.9,
                });
            }
            Ok(faces)
        }
    }
    fn embedding(
        &mut self,
        _: ai::vision::IdentityFrame<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<ai::vision::FaceEmbedding>, ai::Error> {
        let mut values = vec![0.; 128];
        values[0] = 1.;
        Ok(Some(ai::vision::FaceEmbedding::new(&values)?))
    }
    fn speaker(
        &mut self,
        chunk: SpeakerChunk<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<f32>, ai::Error> {
        Ok(vec![
            if chunk.video()[0] < 100. { 0.9 } else { 0.05 };
            chunk.frames()
        ])
    }
    fn scene(
        &mut self,
        _: SceneFrame<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<SceneResult, ai::Error> {
        Ok(SceneResult {
            kind: SceneKind::Screen,
            screen_probability: Some(0.9),
        })
    }
}
#[test]
fn analysis_partitions_shots_keeps_identity_local_and_reuses_only_matching_models() {
    let Some(asset) = fixture() else {
        return;
    };
    let (mut runner, mut cache, _dir) = setup();
    let request = AnalysisRequest::new(
        asset.clone(),
        range(0, 5),
        vec![range(0, 5)],
        [1; 32],
        AnalysisSettings::default(),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let job = request
        .submit(&mut runner, &mut cache, Detector::normal(calls.clone()))
        .unwrap();
    assert!(job.finish(&mut runner, &mut cache).unwrap().is_none());
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let result = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(result.shots.len(), 3);
    assert_eq!(
        result.shots.iter().map(|s| s.range).collect::<Vec<_>>(),
        vec![range(0, 2), range(2, 4), range(4, 5)]
    );
    assert!(matches!(
        result.shots[0].speaker,
        SpeakerState::Visible {
            local: SpeakerRef { shot: 0, track: 0 },
            ..
        }
    ));
    assert_eq!(result.shots[1].speaker, SpeakerState::OffScreenSpeech);
    assert_eq!(result.shots[1].tracks[0].id, 0); // Reset, never person identity 0.
    assert_eq!(result.shots[2].speaker, SpeakerState::OffScreenSpeech);
    assert_eq!(result.shots[2].scene, Scene::Screen);
    assert_eq!(
        result
            .shots
            .iter()
            .map(|s| s.people_count)
            .collect::<Vec<_>>(),
        vec![1, 1, 0]
    );
    for shot in &result.shots {
        assert_eq!(shot.continuity_hint, Some(SpeakerRef { shot: 0, track: 0 }));
        assert!(shot.tracks.iter().all(|track| track.identity.is_none()));
    }
    assert_eq!(cache.len(), 1);
    let before = calls.load(Ordering::SeqCst);
    let reused = request
        .submit(
            &mut runner,
            &mut cache,
            Detector {
                mode: 3,
                ..Detector::normal(calls.clone())
            },
        )
        .unwrap();
    assert_eq!(wait(&mut runner, reused.id), JobState::Completed);
    assert_eq!(
        reused.finish(&mut runner, &mut cache).unwrap().unwrap(),
        result
    );
    assert_eq!(calls.load(Ordering::SeqCst), before);
    let changed = AnalysisRequest::new(
        asset,
        range(0, 5),
        vec![range(0, 5)],
        [2; 32],
        AnalysisSettings::default(),
    )
    .unwrap();
    assert_ne!(changed.cache_key(), request.cache_key());
    let bytes = cache
        .get(request.cache_key())
        .unwrap()
        .unwrap()
        .bytes()
        .to_vec();
    cache.put(changed.cache_key(), &bytes).unwrap();
    assert!(matches!(
        changed.submit(&mut runner, &mut cache, Detector::normal(calls)),
        Err(EditError::CacheCorrupt)
    ));
}
#[test]
fn cancelled_failed_panicking_and_invalid_detectors_publish_nothing() {
    let Some(asset) = fixture() else {
        return;
    };
    for mode in [1, 2, 3, 4, 5] {
        let (mut runner, mut cache, _dir) = setup();
        let request = AnalysisRequest::new(
            asset.clone(),
            range(0, if mode == 2 || mode == 5 { 4 } else { 2 }),
            vec![],
            [1; 32],
            AnalysisSettings::default(),
        )
        .unwrap();
        let (started, receive) = mpsc::sync_channel(1);
        let job = request
            .submit(
                &mut runner,
                &mut cache,
                Detector {
                    mode,
                    calls: Arc::new(AtomicUsize::new(0)),
                    started: Some(started),
                },
            )
            .unwrap();
        runner.tick().unwrap();
        if mode == 2 {
            receive.recv_timeout(Duration::from_secs(120)).unwrap();
            job.cancel.cancel();
        }
        assert_eq!(
            wait(&mut runner, job.id),
            if mode == 2 {
                JobState::Cancelled
            } else {
                JobState::Failed
            }
        );
        assert!(job.finish(&mut runner, &mut cache).is_err());
        assert!(runner.output_bytes(job.id).is_none());
        assert!(cache.is_empty());
    }
}
#[test]
fn request_refuses_hostile_settings_and_speech_before_submission() {
    let Some(asset) = fixture() else {
        return;
    };
    for settings in [
        AnalysisSettings {
            cut_threshold: f32::NAN,
            ..Default::default()
        },
        AnalysisSettings {
            cut_threshold: 0.,
            ..Default::default()
        },
        AnalysisSettings {
            min_shot: Tick::ZERO,
            ..Default::default()
        },
    ] {
        assert!(
            AnalysisRequest::new(asset.clone(), range(0, 5), vec![], [1; 32], settings).is_err()
        );
    }
    for speech in [
        vec![range(2, 4), range(1, 2)],
        vec![range(0, 3), range(2, 4)],
        vec![range(4, 6)],
    ] {
        assert!(
            AnalysisRequest::new(
                asset.clone(),
                range(0, 5),
                speech,
                [1; 32],
                AnalysisSettings::default()
            )
            .is_err()
        );
    }
    assert!(
        AnalysisRequest::new(
            asset,
            range(0, 6),
            vec![],
            [1; 32],
            AnalysisSettings::default()
        )
        .is_err()
    );
}

#[test]
fn short_and_audio_absent_evidence_stays_unknown_and_pre_cancel_never_runs() {
    let Some(asset) = fixture() else {
        return;
    };
    let (mut runner, mut cache, _dir) = setup();
    let calls = Arc::new(AtomicUsize::new(0));
    // Visible evidence is independent of people-count dwell and ASD coverage.
    for (start_ms, duration_ms, mode, expected, people, second_states) in [
        (
            0,
            400,
            0,
            SpeakerState::Unknown,
            0,
            vec![SpeakerState::Unknown],
        ),
        (
            0,
            480,
            0,
            SpeakerState::Unknown,
            0,
            vec![SpeakerState::Unknown],
        ),
        (
            0,
            520,
            0,
            SpeakerState::Unknown,
            1,
            vec![SpeakerState::Unknown],
        ),
        (
            0,
            600,
            0,
            SpeakerState::Unknown,
            1,
            vec![SpeakerState::Unknown],
        ),
        (
            2000,
            2000,
            7,
            SpeakerState::Unknown,
            1,
            vec![SpeakerState::Unknown; 2],
        ),
        (
            2000,
            2000,
            6,
            SpeakerState::Unknown,
            1,
            vec![SpeakerState::Unknown, SpeakerState::OffScreenSpeech],
        ),
        (
            4000,
            400,
            0,
            SpeakerState::OffScreenSpeech,
            0,
            vec![SpeakerState::OffScreenSpeech],
        ),
        (
            4000,
            480,
            0,
            SpeakerState::OffScreenSpeech,
            0,
            vec![SpeakerState::OffScreenSpeech],
        ),
    ] {
        let start = Tick(TICKS_PER_SECOND * start_ms / 1000);
        let short =
            SourceRange::new(start, Tick(start.0 + TICKS_PER_SECOND * duration_ms / 1000)).unwrap();
        let request = AnalysisRequest::new(
            asset.clone(),
            short,
            vec![short],
            [mode + 1; 32],
            AnalysisSettings::default(),
        )
        .unwrap();
        let job = request
            .submit(
                &mut runner,
                &mut cache,
                Detector {
                    mode,
                    ..Detector::normal(Arc::new(AtomicUsize::new(0)))
                },
            )
            .unwrap();
        assert_eq!(wait(&mut runner, job.id), JobState::Completed);
        let result = job.finish(&mut runner, &mut cache).unwrap().unwrap();
        let shot = &result.shots[0];
        assert_eq!(
            shot.speaker, expected,
            "start={start_ms} duration={duration_ms} mode={mode}"
        );
        assert_eq!(
            shot.people_count, people,
            "start={start_ms} duration={duration_ms} mode={mode}"
        );
        assert_eq!(
            shot.seconds
                .iter()
                .map(|second| second.speaker)
                .collect::<Vec<_>>(),
            second_states,
            "start={start_ms} duration={duration_ms} mode={mode}"
        );
    }
    let limited = media::VisionPrepLimits {
        max_frame_pixels: 4095,
        ..Default::default()
    };
    assert!(matches!(
        media::decode_frame_at(
            &asset,
            Tick::ZERO,
            &limited,
            Arc::new(std::sync::atomic::AtomicBool::new(false))
        ),
        Err(media::MediaError::TooLarge(_))
    ));
    let delayed_path = asset.path().with_file_name("late-audio.mkv");
    let output = Command::new("ffmpeg")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(asset.path())
        .args(["-itsoffset", "6", "-i"])
        .arg(asset.path())
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"])
        .arg(&delayed_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let delayed = SourceAsset::import(AssetId(3), delayed_path).unwrap();
    let delayed_request = AnalysisRequest::new(
        delayed,
        range(4, 5),
        vec![range(4, 5)],
        [1; 32],
        AnalysisSettings::default(),
    )
    .unwrap();
    let delayed_job = delayed_request
        .submit(&mut runner, &mut cache, Detector::normal(calls.clone()))
        .unwrap();
    assert_eq!(wait(&mut runner, delayed_job.id), JobState::Completed);
    assert_eq!(
        delayed_job
            .finish(&mut runner, &mut cache)
            .unwrap()
            .unwrap()
            .shots[0]
            .speaker,
        SpeakerState::Unknown
    );
    let path = asset.path().with_file_name("video-only.mkv");
    let output = Command::new("ffmpeg")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(asset.path())
        .args(["-an", "-c:v", "copy"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let silent = SourceAsset::import(AssetId(2), path).unwrap();
    let request = AnalysisRequest::new(
        silent,
        range(0, 2),
        vec![range(0, 2)],
        [1; 32],
        AnalysisSettings::default(),
    )
    .unwrap();
    let job = request
        .submit(&mut runner, &mut cache, Detector::normal(calls.clone()))
        .unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    assert_eq!(
        job.finish(&mut runner, &mut cache).unwrap().unwrap().shots[0].speaker,
        SpeakerState::Unknown
    );
    let job = request
        .submit(&mut runner, &mut cache, Detector::normal(calls.clone()))
        .unwrap();
    let before = calls.load(Ordering::SeqCst);
    job.cancel.cancel();
    assert_eq!(wait(&mut runner, job.id), JobState::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), before);
    assert!(matches!(
        job.finish(&mut runner, &mut cache),
        Err(EditError::Cancelled)
    ));
}

#[cfg(feature = "vision")]
#[test]
#[ignore = "requires verified private vision models and permitted 30-minute source"]
fn real_vision_analysis_smoke() {
    struct Hooks;
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn stage(&self, _: ai::Stage) {}
    }
    let weights = PathBuf::from(std::env::var_os("VIBECUT_VISION_MODELS").unwrap());
    let source = PathBuf::from(std::env::var_os("VIBECUT_VISION_SOURCE").unwrap());
    let asset = SourceAsset::import(AssetId(1), source).unwrap();
    let start = std::env::var("VIBECUT_VISION_START")
        .unwrap_or("100".into())
        .parse::<i64>()
        .unwrap();
    let duration = std::env::var("VIBECUT_VISION_DURATION")
        .unwrap_or("3".into())
        .parse::<i64>()
        .unwrap();
    let started = Instant::now();
    let detector = ai::vision::TractVision::load(
        ai::vision::VisionPaths {
            yunet: &weights,
            clip: &weights,
            light_asd: &weights,
        },
        1,
        6,
        &Hooks,
    )
    .unwrap();
    let fingerprint = *detector.fingerprint().unwrap().as_bytes();
    let (mut runner, mut cache, _dir) = setup();
    let request = AnalysisRequest::new(
        asset,
        range(start, start + duration),
        vec![],
        fingerprint,
        AnalysisSettings::default(),
    )
    .unwrap();
    let job = request.submit(&mut runner, &mut cache, detector).unwrap();
    assert_eq!(
        wait(&mut runner, job.id),
        JobState::Completed,
        "{:?}",
        runner.snapshot(job.id).unwrap().error
    );
    let result = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    println!(
        "REAL_VISION elapsed_ms={} shots={} bins={} tracks={}",
        started.elapsed().as_millis(),
        result.shots.len(),
        result.shots.iter().map(|s| s.seconds.len()).sum::<usize>(),
        result.shots.iter().map(|s| s.tracks.len()).sum::<usize>()
    );
    for shot in result.shots {
        println!(
            "SHOT {} {} people={} speaker={:?} scene={:?}",
            shot.range.start().0,
            shot.range.end().0,
            shot.people_count,
            shot.speaker,
            shot.scene
        );
    }
}

#[test]
fn cache_pressure_keeps_complete_output_for_finish_retry() {
    let Some(asset) = fixture() else {
        return;
    };
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (mut runner, mut cache, _dir) = setup_budget(budget.clone());
    let request = AnalysisRequest::new(
        asset,
        range(0, 2),
        vec![],
        [1; 32],
        AnalysisSettings::default(),
    )
    .unwrap();
    let job = request
        .submit(
            &mut runner,
            &mut cache,
            Detector::normal(Arc::new(AtomicUsize::new(0))),
        )
        .unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let snapshot = budget.snapshot().unwrap();
    let pressure = budget
        .reserve(
            engine::ByteOwner::DecodedFrames,
            snapshot.limit - snapshot.used - 100,
        )
        .unwrap();
    assert!(matches!(
        job.finish(&mut runner, &mut cache),
        Err(EditError::BudgetFull)
    ));
    assert!(runner.output_bytes(job.id).is_some());
    assert!(cache.is_empty());
    drop(pressure);
    assert!(job.finish(&mut runner, &mut cache).unwrap().is_some());
    assert_eq!(cache.len(), 1);
}

#[test]
fn repeated_faces_gain_cross_shot_identity_and_survive_cache_reopen() {
    let Some(asset) = fixture() else {
        return;
    };
    let (mut runner, mut cache, _dir) = setup();
    let request = AnalysisRequest::new(
        asset,
        range(0, 5),
        vec![],
        [8; 32],
        AnalysisSettings::default(),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let job = request
        .submit(
            &mut runner,
            &mut cache,
            Detector {
                mode: 8,
                ..Detector::normal(calls.clone())
            },
        )
        .unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let result = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(result.shots[0].tracks[0].identity, None);
    assert_eq!(
        result.shots[1].tracks[0].identity,
        Some(SpeakerRef { shot: 0, track: 0 })
    );
    let reused = request
        .submit(
            &mut runner,
            &mut cache,
            Detector {
                mode: 3,
                ..Detector::normal(calls.clone())
            },
        )
        .unwrap();
    assert_eq!(wait(&mut runner, reused.id), JobState::Completed);
    assert_eq!(
        reused.finish(&mut runner, &mut cache).unwrap().unwrap(),
        result
    );
    let mut corrupt: serde_json::Value =
        serde_json::from_slice(cache.get(request.cache_key()).unwrap().unwrap().bytes()).unwrap();
    corrupt["shots"][1]["tracks"][0]["identity"]["shot"] = serde_json::json!(1);
    cache
        .put(request.cache_key(), &serde_json::to_vec(&corrupt).unwrap())
        .unwrap();
    assert!(matches!(
        request.submit(&mut runner, &mut cache, Detector::normal(calls)),
        Err(EditError::CacheCorrupt)
    ));
}

#[test]
fn same_histogram_spatial_cut_retains_rgb_only_shot_boundaries() {
    let asset = super::scene_detection::fixture("same-histogram", 64, 20, false);
    let source = range(0, 2);
    let (mut runner, mut cache, _dir) = setup();
    let scene_request =
        engine::scene_detection::DetectionRequest::new(asset.clone(), source, Default::default())
            .unwrap();
    let scene_job = scene_request.submit(&mut runner, &mut cache).unwrap();
    assert_eq!(wait(&mut runner, scene_job.id), JobState::Completed);
    assert!(
        scene_job
            .finish(&mut runner, &mut cache)
            .unwrap()
            .unwrap()
            .candidates
            .is_empty()
    );
    let request =
        AnalysisRequest::new(asset, source, vec![], [1; 32], AnalysisSettings::default()).unwrap();
    let job = request
        .submit(
            &mut runner,
            &mut cache,
            Detector::normal(Arc::new(AtomicUsize::new(0))),
        )
        .unwrap();
    assert_eq!(wait(&mut runner, job.id), JobState::Completed);
    let result = job.finish(&mut runner, &mut cache).unwrap().unwrap();
    assert_eq!(
        result.shots.iter().map(|s| s.range).collect::<Vec<_>>(),
        vec![range(0, 1), range(1, 2)]
    );
}

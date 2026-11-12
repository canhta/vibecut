//! Verifies timed-word mapping, the consent gate and typed model-ID refusals.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::fillers::{FillerJob, FillerRequest, decode};
use engine::jobs::{JobRunner, JobState, RunnerConfig, SystemClock};
use engine::providers::{ConsentPrompt, TextAccess, UploadRequest};
use engine::{AnalysisError, ByteBudget};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use time::{SourceRange, Tick};

fn cue(id: &str, start: i64, end: i64, text: &str, words: &[(i64, i64)]) -> captions::Cue {
    captions::Cue {
        word_timing: captions::CueWordTiming::Unverified,
        id: id.into(),
        range: SourceRange::new(Tick(start), Tick(end)).unwrap(),
        source_lang: captions::LanguageTag::parse("en").unwrap(),
        texts: [(
            captions::LanguageTag::parse("en").unwrap(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        )]
        .into_iter()
        .collect(),
        word_ranges: words
            .iter()
            .map(|(start, end)| SourceRange::new(Tick(*start), Tick(*end)).unwrap())
            .collect(),
        style: None,
    }
}

/// Two caption cues with measured words: `Um brave` then `like`.
fn project() -> Arc<Project> {
    let mut data = Project::empty(ProjectId(1), "Fillers".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(0, std::env::temp_dir().join("vibecut-fillers-source.mp4"))
            .format("mp4")
            .duration(Tick(300))
            .content_hash(media::ContentHash::from_bytes(b"fillers"))
            .stream(
                testkit::source::video()
                    .time_base(media::TimeBase::new(1, 30000).unwrap())
                    .duration(Tick(300))
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.overlays.push(Track {
        id: TrackId(4),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(4),
            animations: Default::default(),
            source_asset: Some(media::AssetId(0)),
            source_range: SourceRange::new(Tick::ZERO, Tick(300)).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Caption(CaptionParams {
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues: vec![
                        cue("cue-1", 0, 200, "Um brave", &[(0, 100), (100, 200)]),
                        cue("cue-2", 200, 300, "like", &[(200, 300)]),
                    ],
                },
            }),
        }],
    });
    Arc::new(Project::new(data).unwrap())
}

fn config() -> ai::ProviderConfig {
    ai::fillers::OpenAiFillerClassifier::provider_config("gpt-6-luna").unwrap()
}

struct Fillers {
    findings: Vec<ai::fillers::FillerFinding>,
    calls: AtomicUsize,
    seen: Mutex<Option<ai::fillers::FillerInput>>,
    wait_for_cancel: bool,
}
impl ai::fillers::FillerClassifier for Fillers {
    fn config(&self) -> &ai::ProviderConfig {
        static CONFIG: std::sync::OnceLock<ai::ProviderConfig> = std::sync::OnceLock::new();
        CONFIG.get_or_init(config)
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config().model().clone(),
            mode: ai::fillers::MODE.into(),
            operation: ai::Operation::ClassifyFillers,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::fillers::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "fillers-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        input: &ai::fillers::FillerInput,
        _: &ai::Request<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> Result<ai::fillers::FillerProposal, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().unwrap() = Some(input.clone());
        if self.wait_for_cancel {
            while !hooks.cancelled() {
                std::thread::yield_now();
            }
            return Err(ai::Error::Cancelled);
        }
        Ok(ai::fillers::FillerProposal {
            fillers: self.findings.clone(),
        })
    }
}

struct UnusedChapters;
impl ai::chapters::ChapterGenerator for UnusedChapters {
    fn config(&self) -> &ai::ProviderConfig {
        static CONFIG: std::sync::OnceLock<ai::ProviderConfig> = std::sync::OnceLock::new();
        CONFIG.get_or_init(config)
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        panic!("chapter capabilities are unused")
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        _: &ai::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::chapters::ChapterProposal, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}

struct Grant {
    allowed: Arc<AtomicBool>,
}
impl ConsentPrompt for Grant {
    fn approve(&self, _: &UploadRequest) -> engine::providers::Result<()> {
        if self.allowed.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(engine::providers::Error::ConsentRequired.into())
        }
    }
}

fn access(
    classifier: Arc<dyn ai::fillers::FillerClassifier>,
    allowed: Arc<AtomicBool>,
) -> TextAccess {
    TextAccess::new(
        Arc::new(UnusedChapters),
        classifier,
        Box::new(Grant { allowed }),
    )
}

fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 8,
            retained: 8,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(1),
            progress_interval: std::time::Duration::from_millis(10),
        },
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}

fn wait(runner: &mut JobRunner, job: &FillerJob) -> JobState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(job.id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return state;
        }
        assert!(std::time::Instant::now() < deadline, "job deadline");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn classification_maps_exact_word_ids_to_output_candidates() {
    let project = project();
    let before = serde_json::to_vec(project.data()).unwrap();
    let findings = vec![
        ai::fillers::FillerFinding {
            id: "filler:4:cue-1:100".into(),
            kind: captions::FillerKind::FilledPause,
        },
        ai::fillers::FillerFinding {
            id: "filler:4:cue-2:200".into(),
            kind: captions::FillerKind::DiscourseMarker,
        },
    ];
    let classifier = Arc::new(Fillers {
        findings,
        calls: AtomicUsize::new(0),
        seen: Mutex::new(None),
        wait_for_cancel: false,
    });
    let allowed = Arc::new(AtomicBool::new(true));
    let access = access(classifier.clone(), allowed);
    let request =
        FillerRequest::new(&project, captions::LanguageTag::parse("en").unwrap(), true).unwrap();
    let sent = request.input().clone();
    assert_eq!(sent.segments.len(), 2);
    let mut runner = runner();
    let job = request.submit(&mut runner, &access).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    let candidates = job.finish(&mut runner).unwrap().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].id.as_str(), "filler:4:cue-1:100");
    assert_eq!(candidates[0].range.start, Tick(100));
    assert_eq!(candidates[0].range.end, Tick(200));
    assert_eq!(candidates[0].kind, captions::FillerKind::FilledPause);
    assert!(!candidates[0].edited_cue);
    assert_eq!(candidates[1].id.as_str(), "filler:4:cue-2:200");
    assert_eq!(candidates[1].kind, captions::FillerKind::DiscourseMarker);
    assert_eq!(serde_json::to_vec(project.data()).unwrap(), before);
    let seen = classifier.seen.lock().unwrap().clone().unwrap();
    assert_eq!(seen, sent);
    assert_eq!(
        serde_json::from_slice::<Vec<engine::control::FillerCandidate>>(
            &serde_json::to_vec(&candidates).unwrap()
        )
        .unwrap(),
        candidates
    );
    assert!(decode(&serde_json::to_vec(&candidates).unwrap()).is_ok());
}

#[test]
fn unknown_duplicate_and_disallowed_marker_ids_are_typed_refusals() {
    for findings in [
        vec![ai::fillers::FillerFinding {
            id: "filler:4:cue-1:999".into(),
            kind: captions::FillerKind::FilledPause,
        }],
        vec![
            ai::fillers::FillerFinding {
                id: "filler:4:cue-1:0".into(),
                kind: captions::FillerKind::FilledPause,
            },
            ai::fillers::FillerFinding {
                id: "filler:4:cue-1:0".into(),
                kind: captions::FillerKind::FilledPause,
            },
        ],
        vec![ai::fillers::FillerFinding {
            id: "filler:4:cue-1:0".into(),
            kind: captions::FillerKind::DiscourseMarker,
        }],
    ] {
        let project = project();
        let classifier = Arc::new(Fillers {
            findings,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(None),
            wait_for_cancel: false,
        });
        let access = access(classifier, Arc::new(AtomicBool::new(true)));
        let request =
            FillerRequest::new(&project, captions::LanguageTag::parse("en").unwrap(), false)
                .unwrap();
        let mut runner = runner();
        let job = request.submit(&mut runner, &access).unwrap();
        assert_eq!(wait(&mut runner, &job), JobState::Failed);
        assert!(matches!(
            job.finish(&mut runner),
            Err(AnalysisError::Runtime(engine::RuntimeError::Failed(error)))
                if matches!((error.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<AnalysisError>(), Some(AnalysisError::Ai(ai::Error::Fillers(ai::fillers::FillerRefusal::InvalidSegments))
                ))
        ));
    }
}

#[test]
fn consent_is_required_before_the_adapter_and_cancel_discards_the_result() {
    let project = project();
    let classifier = Arc::new(Fillers {
        findings: vec![ai::fillers::FillerFinding {
            id: "filler:4:cue-1:0".into(),
            kind: captions::FillerKind::FilledPause,
        }],
        calls: AtomicUsize::new(0),
        seen: Mutex::new(None),
        wait_for_cancel: true,
    });
    let allowed = Arc::new(AtomicBool::new(false));
    let access = access(classifier.clone(), allowed.clone());
    let request =
        FillerRequest::new(&project, captions::LanguageTag::parse("en").unwrap(), false).unwrap();
    let mut runner = runner();
    assert!(matches!(
        request.submit(&mut runner, &access),
        Err(AnalysisError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    assert_eq!(classifier.calls.load(Ordering::SeqCst), 0);

    allowed.store(true, Ordering::SeqCst);
    let job = request.submit(&mut runner, &access).unwrap();
    runner.cancel_job(job.id).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Cancelled);
    assert!(matches!(
        job.finish(&mut runner),
        Err(AnalysisError::Runtime(engine::RuntimeError::Cancelled))
    ));
}

#[test]
fn empty_unsupported_and_untimed_transcripts_refuse_before_a_job() {
    let empty = Arc::new(Project::empty(ProjectId(1), "Empty".into()).unwrap());
    assert!(matches!(
        FillerRequest::new(&empty, captions::LanguageTag::parse("en").unwrap(), false),
        Err(AnalysisError::Ai(ai::Error::Fillers(
            ai::fillers::FillerRefusal::EmptyTranscript
        )))
    ));
    assert!(matches!(
        FillerRequest::new(&empty, captions::LanguageTag::parse("fr").unwrap(), false),
        Err(AnalysisError::Ai(ai::Error::Fillers(
            ai::fillers::FillerRefusal::UnsupportedLanguage
        )))
    ));
    let project = project();
    assert!(matches!(
        FillerRequest::new(&project, captions::LanguageTag::parse("vi").unwrap(), false),
        Err(AnalysisError::Ai(ai::Error::Fillers(
            ai::fillers::FillerRefusal::EmptyTranscript
        )))
    ));
}

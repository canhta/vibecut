//! Verifies verified-word cue selection, the consent gate, typed refusals and that a provider
//! failure never falls back to the lexicon.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::emphasis::{EmphasisJob, EmphasisRequest, decode};
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

const TEXT: &str = "Quarter revenue grew 40 percent";

fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(Tick(start), Tick(end)).unwrap()
}
fn en() -> captions::LanguageTag {
    captions::LanguageTag::parse("en").unwrap()
}
fn cue(id: &str, start: i64, end: i64, text: &str, verified: bool) -> captions::Cue {
    let mut at = 0;
    let records = text
        .split(' ')
        .enumerate()
        .map(|(index, word)| {
            let bytes = at..at + word.len();
            at += word.len() + 1;
            let timed = range(start + index as i64 * 10, start + index as i64 * 10 + 10);
            captions::VerifiedCueWord {
                bytes,
                source_range: timed,
                projection: captions::WordProjection::Visible(timed),
                provenance: captions::TimingProvenance::Native,
            }
        })
        .collect();
    captions::Cue {
        word_timing: if verified {
            captions::CueWordTiming::Words {
                language: en(),
                text: text.into(),
                records,
            }
        } else {
            captions::CueWordTiming::Unverified
        },
        id: id.into(),
        range: range(start, end),
        source_lang: en(),
        texts: [(
            en(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        )]
        .into_iter()
        .collect(),
        word_ranges: Vec::new(),
        style: None,
    }
}

/// `cue-1` has verified words; `cue-2` has none.
fn project() -> Arc<Project> {
    let mut data = Project::empty(ProjectId(1), "Emphasis".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(0, std::env::temp_dir().join("vibecut-emphasis-source.mp4"))
            .format("mp4")
            .duration(Tick(300))
            .content_hash(media::ContentHash::from_bytes(b"emphasis"))
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
            source_range: range(0, 300),
            start: Tick::ZERO,
            content: ItemParams::Caption(CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues: vec![
                        cue("cue-1", 0, 200, TEXT, true),
                        cue("cue-2", 200, 300, "like", false),
                    ],
                },
            }),
        }],
    });
    Arc::new(Project::new(data).unwrap())
}

fn config() -> ai::ProviderConfig {
    ai::emphasis::OpenAiEmphasisSelector::provider_config("gpt-6-luna").unwrap()
}

struct Selector {
    proposal: Result<Vec<ai::emphasis::EmphasisSelection>, ()>,
    calls: AtomicUsize,
    seen: Mutex<Option<ai::emphasis::EmphasisInput>>,
}
impl ai::emphasis::EmphasisSelector for Selector {
    fn config(&self) -> &ai::ProviderConfig {
        static CONFIG: std::sync::OnceLock<ai::ProviderConfig> = std::sync::OnceLock::new();
        CONFIG.get_or_init(config)
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config().model().clone(),
            mode: ai::emphasis::MODE.into(),
            operation: ai::Operation::SelectEmphasis,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::emphasis::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "emphasis-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn select(
        &self,
        input: &ai::emphasis::EmphasisInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::emphasis::EmphasisProposal, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().unwrap() = Some(input.clone());
        match &self.proposal {
            Ok(selections) => Ok(ai::emphasis::EmphasisProposal {
                selections: selections.clone(),
            }),
            Err(()) => Err(ai::Error::Server),
        }
    }
}

struct Unused;
impl ai::chapters::ChapterGenerator for Unused {
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
impl ai::fillers::FillerClassifier for Unused {
    fn config(&self) -> &ai::ProviderConfig {
        static CONFIG: std::sync::OnceLock<ai::ProviderConfig> = std::sync::OnceLock::new();
        CONFIG.get_or_init(config)
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        panic!("filler capabilities are unused")
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        _: &ai::fillers::FillerInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::fillers::FillerProposal, ai::Error> {
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
            Err(engine::providers::Error::ConsentRequired)
        }
    }
}

fn access(selector: Option<Arc<Selector>>, allowed: Arc<AtomicBool>) -> TextAccess {
    let access = TextAccess::new(
        Arc::new(Unused),
        Arc::new(Unused),
        Box::new(Grant { allowed }),
    );
    match selector {
        Some(selector) => access.with_emphasis(selector),
        None => access,
    }
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

fn wait(runner: &mut JobRunner, job: &EmphasisJob) -> JobState {
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

fn selector(proposal: Result<Vec<ai::emphasis::EmphasisSelection>, ()>) -> Arc<Selector> {
    Arc::new(Selector {
        proposal,
        calls: AtomicUsize::new(0),
        seen: Mutex::new(None),
    })
}
fn choose(segment: &str, positions: Vec<u32>) -> ai::emphasis::EmphasisSelection {
    ai::emphasis::EmphasisSelection {
        segment: segment.into(),
        positions,
    }
}

#[test]
fn suggestions_map_positions_to_cues_and_never_mutate_the_project() {
    let project = project();
    let before = serde_json::to_vec(project.data()).unwrap();
    let selector = selector(Ok(vec![choose("4:cue-1", vec![3, 1])]));
    let access = access(Some(selector.clone()), Arc::new(AtomicBool::new(true)));
    let request = EmphasisRequest::new(&project, ItemId(4), None).unwrap();
    // The unverified cue is not sent at all.
    let sent = request.input().clone();
    assert_eq!(sent.segments.len(), 1);
    assert_eq!(sent.segments[0].id.as_str(), "4:cue-1");
    assert_eq!(sent.segments[0].words.len(), 5);
    let mut runner = runner();
    let job = request.submit(&mut runner, &access).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    let suggestions = job.finish(&mut runner).unwrap().unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].item, ItemId(4));
    assert_eq!(suggestions[0].cue.as_str(), "cue-1");
    assert_eq!(suggestions[0].word_ids, vec![1, 3]);
    assert_eq!(serde_json::to_vec(project.data()).unwrap(), before);
    assert_eq!(selector.seen.lock().unwrap().clone().unwrap(), sent);
    assert!(decode(&serde_json::to_vec(&suggestions).unwrap()).is_ok());
}

#[test]
fn model_positions_outside_the_cue_are_a_typed_refusal_not_a_clamp() {
    for proposal in [
        vec![choose("4:cue-1", vec![5])],
        vec![choose("4:cue-1", vec![2, 2])],
        vec![choose("4:cue-1", vec![0, 1, 2, 3])],
        vec![choose("4:cue-2", vec![0])],
    ] {
        let project = project();
        let access = access(
            Some(selector(Ok(proposal))),
            Arc::new(AtomicBool::new(true)),
        );
        let request = EmphasisRequest::new(&project, ItemId(4), None).unwrap();
        let mut runner = runner();
        let job = request.submit(&mut runner, &access).unwrap();
        assert_eq!(wait(&mut runner, &job), JobState::Failed);
        assert!(matches!(
            job.finish(&mut runner),
            Err(AnalysisError::Runtime(engine::RuntimeError::Failed(error)))
                if matches!((error.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<AnalysisError>(), Some(AnalysisError::Ai(ai::Error::Emphasis(ai::emphasis::EmphasisRefusal::InvalidSegments))))
        ));
    }
}

#[test]
fn provider_failure_fails_the_job_and_never_substitutes_the_lexicon() {
    let project = project();
    let selector = selector(Err(()));
    let access = access(Some(selector.clone()), Arc::new(AtomicBool::new(true)));
    let request = EmphasisRequest::new(&project, ItemId(4), None).unwrap();
    let mut runner = runner();
    let job = request.submit(&mut runner, &access).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Failed);
    assert!(matches!(
        job.finish(&mut runner),
        Err(AnalysisError::Runtime(engine::RuntimeError::Failed(_)))
    ));
    assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
    assert!(
        runner.output_bytes(job.id).is_none(),
        "no result of any kind"
    );
}

#[test]
fn consent_and_a_lent_adapter_are_required_before_any_upload() {
    let project = project();
    let selector = selector(Ok(vec![]));
    let allowed = Arc::new(AtomicBool::new(false));
    let request = EmphasisRequest::new(&project, ItemId(4), None).unwrap();
    let mut runner = runner();
    assert!(matches!(
        request.submit(
            &mut runner,
            &access(Some(selector.clone()), allowed.clone())
        ),
        Err(AnalysisError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    // No adapter lent: refused, the lexicon is not an answer.
    assert!(matches!(
        request.submit(&mut runner, &access(None, Arc::new(AtomicBool::new(true)))),
        Err(AnalysisError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    assert_eq!(selector.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cues_without_verified_timing_or_a_caption_item_refuse_before_a_job() {
    let project = project();
    // Naming a cue without verified words refuses with the typed reason.
    assert!(matches!(
        EmphasisRequest::new(&project, ItemId(4), Some(&["cue-2".into()][..])),
        Err(AnalysisError::Edit(engine::EditError::Captions(
            captions::Error::NoWordTiming
        )))
    ));
    assert!(matches!(
        EmphasisRequest::new(&project, ItemId(4), Some(&["missing".into()][..])),
        Err(AnalysisError::Edit(engine::EditError::Invalid(_)))
    ));
    assert!(matches!(
        EmphasisRequest::new(&project, ItemId(99), None),
        Err(AnalysisError::Edit(engine::EditError::Invalid(_)))
    ));
    // An item whose only cue is unverified has nothing eligible.
    let mut data = project.data().clone();
    let ItemParams::Caption(params) = &mut data.scene_mut().timeline.overlays[0].items[0].content
    else {
        panic!("caption");
    };
    params.cues.cues.remove(0);
    let untimed = Arc::new(Project::new(data).unwrap());
    assert!(matches!(
        EmphasisRequest::new(&untimed, ItemId(4), None),
        Err(AnalysisError::Ai(ai::Error::Emphasis(
            ai::emphasis::EmphasisRefusal::EmptyTranscript
        )))
    ));
}

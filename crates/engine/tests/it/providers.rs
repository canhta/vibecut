#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "integration fixtures"
)]
use ai::*;
use captions::{LanguageTag, NfcText};
use engine::{
    EditError,
    providers::{self, Approval, TranslationAttempt},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}
fn lang(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}
fn config(id: &str, remote: bool) -> ProviderConfig {
    ProviderConfig::new(
        id.into(),
        "fixture".into(),
        "v1".into(),
        if remote {
            Location::Remote {
                backend: id.into(),
                base_url: "https://example.invalid/v1".into(),
                auth_ref: "keychain:fixture".into(),
            }
        } else {
            Location::Local {
                backend: id.into(),
                model_ref: "fixture".into(),
            }
        },
    )
    .unwrap()
}
fn window() -> TranslationWindow {
    TranslationWindow {
        source_language: lang("vi"),
        target_language: lang("en"),
        segments: vec![TranslationSegment {
            id: "s1".into(),
            text: "Xin chào".into(),
        }],
        before: vec![],
        after: vec![],
        glossary: vec![],
        style: "".into(),
    }
}
fn scope(config: ProviderConfig) -> Scope {
    Scope {
        provider: config,
        operation: Operation::Translate,
        payload_fingerprint: "fixture".into(),
        mode: "file".into(),
        texts: vec![window().consent_text().unwrap()],
        source_language: lang("vi"),
        target_language: Some(lang("en")),
        source_samples: SampleRange::new(0, 10).unwrap(),
        sample_rate: 10,
        max_bytes: 32768,
    }
}
struct Fake {
    config: ProviderConfig,
    failure: &'static str,
    log: Arc<Mutex<Vec<String>>>,
}
fn failure(kind: &str) -> Error {
    match kind {
        "auth" => Error::Auth,
        "quota" => Error::Quota,
        "network" => Error::Network,
        "schema" => Error::Schema,
        "payload" => Error::Payload,
        "cancel" => Error::Cancelled,
        "consent" => Error::ConsentRequired,
        "cost" => Error::Cost,
        "unsupported" => Error::Unsupported,
        _ => Error::Server,
    }
}
impl Translator for Fake {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            operation: scope.operation,
            mode: scope.mode.clone(),
            languages: vec![lang("vi")],
            target_languages: vec![lang("en")],
            timing: BTreeSet::new(),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: Evidence::FixtureVerified,
            revision: "test".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::UpperBound(Money::new("USD", 10).unwrap())
    }
    fn translate(
        &self,
        w: &TranslationWindow,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        self.log
            .lock()
            .unwrap()
            .push(self.config.id().as_str().into());
        if !self.failure.is_empty() {
            return Err(failure(self.failure));
        }
        Ok(Translation {
            source_language: w.source_language.clone(),
            target_language: w.target_language.clone(),
            texts: BTreeMap::from([("s1".into(), "Hello".into())]),
            number_warnings: vec![],
        })
    }
}
struct Approve {
    log: Arc<Mutex<Vec<String>>>,
    deny_fallback: bool,
    reuse: Option<Consent>,
    budget: u64,
}
impl Approval for Approve {
    fn approve(
        &mut self,
        scope: &Scope,
        _: &CostEstimate,
        fallback: bool,
    ) -> Result<Option<Consent>, Error> {
        self.log.lock().unwrap().push(format!(
            "approve:{}:{fallback}",
            scope.provider.id().as_str()
        ));
        if fallback && self.deny_fallback {
            return Ok(None);
        }
        if fallback && self.reuse.is_some() {
            return Ok(self.reuse.clone());
        }
        Ok(Some(Consent::grant(
            scope.clone(),
            Money::new("USD", self.budget).unwrap(),
        )?))
    }
}
fn fake(id: &str, error: &'static str, log: &Arc<Mutex<Vec<String>>>) -> Fake {
    Fake {
        config: config(id, true),
        failure: error,
        log: log.clone(),
    }
}
fn approve(log: &Arc<Mutex<Vec<String>>>) -> Approve {
    Approve {
        log: log.clone(),
        deny_fallback: false,
        reuse: None,
        budget: 10,
    }
}
#[test]
fn default_never_falls_back_and_selection_roundtrips_without_secrets_or_approval() {
    let log = Arc::default();
    let primary = fake("primary", "auth", &log);
    let settings = ProviderSelection::new(Operation::Translate, primary.config.clone());
    let s = scope(primary.config.clone());
    let attempts = [TranslationAttempt {
        provider: &primary,
        scope: &s,
    }];
    assert!(matches!(
        providers::translate(
            &settings,
            &attempts,
            &window(),
            &mut approve(&log),
            &Hooks::default()
        ),
        Err(EditError::Ai(Error::Auth))
    ));
    assert_eq!(*log.lock().unwrap(), ["approve:primary:false", "primary"]);
    let mut json = serde_json::to_value(&settings).unwrap();
    json.as_object_mut().unwrap().remove("fallbacks");
    let decoded: ProviderSelection = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(decoded, settings);
    json["schema_version"] = 2.into();
    assert!(serde_json::from_value::<ProviderSelection>(json).is_err());
    assert!(
        settings
            .clone()
            .with_fallbacks(vec![primary.config.clone()])
            .is_err()
    );
    assert!(
        settings
            .with_fallbacks((0..8).map(|n| config(&format!("p{n}"), true)).collect())
            .is_err()
    );
}
#[test]
fn configured_order_only_and_exact_success_provenance() {
    for error in ["auth", "quota", "network", "schema"] {
        let log = Arc::default();
        let p = fake("primary", error, &log);
        let f = fake("fallback", "schema", &log);
        let last = fake("last", "", &log);
        let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
            .with_fallbacks(vec![f.config.clone(), last.config.clone()])
            .unwrap();
        let scopes = [
            scope(p.config.clone()),
            scope(f.config.clone()),
            scope(last.config.clone()),
        ];
        let attempts = [
            TranslationAttempt {
                provider: &p,
                scope: &scopes[0],
            },
            TranslationAttempt {
                provider: &f,
                scope: &scopes[1],
            },
            TranslationAttempt {
                provider: &last,
                scope: &scopes[2],
            },
        ];
        let output = providers::translate(
            &settings,
            &attempts,
            &window(),
            &mut approve(&log),
            &Hooks::default(),
        )
        .unwrap();
        assert_eq!(output.provider, last.config);
        assert_eq!(output.output.texts[&NfcText::from("s1")].as_str(), "Hello");
        assert_eq!(
            *log.lock().unwrap(),
            [
                "approve:primary:false",
                "primary",
                "approve:fallback:true",
                "fallback",
                "approve:last:true",
                "last"
            ]
        );
    }
}
#[test]
fn denied_fallback_and_primary_token_reuse_never_send_to_new_provider() {
    for reuse in [false, true] {
        let log = Arc::default();
        let p = fake("primary", "network", &log);
        let f = fake("fallback", "", &log);
        let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
            .with_fallbacks(vec![f.config.clone()])
            .unwrap();
        let a = scope(p.config.clone());
        let b = scope(f.config.clone());
        let attempts = [
            TranslationAttempt {
                provider: &p,
                scope: &a,
            },
            TranslationAttempt {
                provider: &f,
                scope: &b,
            },
        ];
        let mut approval = approve(&log);
        approval.deny_fallback = !reuse;
        if reuse {
            approval.reuse =
                Some(Consent::grant(a.clone(), Money::new("USD", 10).unwrap()).unwrap());
        }
        assert!(matches!(
            providers::translate(
                &settings,
                &attempts,
                &window(),
                &mut approval,
                &Hooks::default()
            ),
            Err(EditError::Ai(Error::ConsentRequired))
        ));
        assert_eq!(
            *log.lock().unwrap(),
            ["approve:primary:false", "primary", "approve:fallback:true"]
        );
    }
}
#[test]
fn refusal_cost_cancellation_and_input_errors_never_trigger_fallback() {
    for error in ["payload", "cancel", "consent", "cost", "unsupported"] {
        let log = Arc::default();
        let p = fake("primary", error, &log);
        let f = fake("fallback", "", &log);
        let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
            .with_fallbacks(vec![f.config.clone()])
            .unwrap();
        let a = scope(p.config.clone());
        let b = scope(f.config.clone());
        let attempts = [
            TranslationAttempt {
                provider: &p,
                scope: &a,
            },
            TranslationAttempt {
                provider: &f,
                scope: &b,
            },
        ];
        assert!(
            providers::translate(
                &settings,
                &attempts,
                &window(),
                &mut approve(&log),
                &Hooks::default()
            )
            .is_err()
        );
        assert_eq!(*log.lock().unwrap(), ["approve:primary:false", "primary"]);
    }
}
#[test]
fn unconfigured_or_reordered_attempts_and_wrong_feature_refuse_before_approval() {
    let log = Arc::default();
    let p = fake("primary", "", &log);
    let f = fake("fallback", "", &log);
    let a = scope(p.config.clone());
    let b = scope(f.config.clone());
    let attempts = [
        TranslationAttempt {
            provider: &f,
            scope: &b,
        },
        TranslationAttempt {
            provider: &p,
            scope: &a,
        },
    ];
    let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
        .with_fallbacks(vec![f.config.clone()])
        .unwrap();
    assert!(
        providers::translate(
            &settings,
            &attempts,
            &window(),
            &mut approve(&log),
            &Hooks::default()
        )
        .is_err()
    );
    let settings = ProviderSelection::new(Operation::SuggestClips, p.config.clone());
    assert!(
        providers::translate(
            &settings,
            &attempts[1..],
            &window(),
            &mut approve(&log),
            &Hooks::default()
        )
        .is_err()
    );
    assert!(log.lock().unwrap().is_empty());
}

impl Transcriber for Fake {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, s: &Scope) -> Capabilities {
        Translator::capabilities(self, s)
    }
    fn estimate(&self, s: &Scope) -> CostEstimate {
        Translator::estimate(self, s)
    }
    fn transcribe(
        &self,
        _: AudioWindow<'_>,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.log
            .lock()
            .unwrap()
            .push(self.config.id().as_str().into());
        if self.failure.is_empty() {
            Ok(vec![])
        } else {
            Err(failure(self.failure))
        }
    }
}
#[test]
fn transcription_falls_back_once_and_retains_successful_recognizer() {
    let log = Arc::default();
    let p = fake("primary", "schema", &log);
    let f = fake("fallback", "", &log);
    let settings = ProviderSelection::new(Operation::Transcribe, p.config.clone())
        .with_fallbacks(vec![f.config.clone()])
        .unwrap();
    let scopes: Vec<_> = [&p, &f]
        .iter()
        .map(|p| {
            let mut s = scope(p.config.clone());
            s.operation = Operation::Transcribe;
            s.target_language = None;
            s.texts.clear();
            s
        })
        .collect();
    let samples = [0.0; 10];
    let attempts = [
        providers::TranscriptionAttempt {
            provider: &p,
            scope: &scopes[0],
            audio: AudioWindow {
                samples: &samples,
                range: scopes[0].source_samples,
                sample_rate: 10,
            },
        },
        providers::TranscriptionAttempt {
            provider: &f,
            scope: &scopes[1],
            audio: AudioWindow {
                samples: &samples,
                range: scopes[1].source_samples,
                sample_rate: 10,
            },
        },
    ];
    let result =
        providers::transcribe(&settings, &attempts, &mut approve(&log), &Hooks::default()).unwrap();
    assert_eq!(result.provider, f.config);
    assert_eq!(result.output.fingerprint().provider, *f.config.id());
    assert!(result.output.segments().is_empty());
}
impl ai::clips::ClipSuggester for Fake {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, s: &Scope) -> Capabilities {
        Translator::capabilities(self, s)
    }
    fn estimate(&self, s: &Scope) -> CostEstimate {
        Translator::estimate(self, s)
    }
    fn source(&self) -> ai::clips::ClipSource {
        ai::clips::ClipSource::Llm
    }
    fn suggest(
        &self,
        _: &ai::clips::ClipInput,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<ai::clips::ClipProposal>, Error> {
        self.log
            .lock()
            .unwrap()
            .push(self.config.id().as_str().into());
        if self.failure.is_empty() {
            Ok(vec![])
        } else {
            Err(failure(self.failure))
        }
    }
}

#[test]
fn clips_use_local_heuristics_only_when_explicitly_configured() {
    use ai::clips::*;
    use time::{SourceRange, TICKS_PER_SECOND, Tick};
    let log = Arc::default();
    let p = fake("primary", "network", &log);
    let local = HeuristicClipSuggester::new().unwrap();
    let input = ClipInput {
        segments: vec![ClipSegment {
            id: "s1".into(),
            text: "Xin chào.".into(),
            range: SourceRange::new(Tick::ZERO, Tick(4 * TICKS_PER_SECOND)).unwrap(),
        }],
        settings: ClipSettings {
            max_segments: 4,
            min_segment_duration: Tick(TICKS_PER_SECOND),
            count: 1,
            min_duration: Tick(TICKS_PER_SECOND),
            max_duration: Tick(5 * TICKS_PER_SECOND),
            overlap_tolerance: 0.0,
            language: lang("vi"),
            keywords: vec![],
        },
    };
    let clip_scope = |cfg| {
        let mut s = scope(cfg);
        s.operation = Operation::SuggestClips;
        s.target_language = None;
        s.mode = MODE.into();
        s.texts = vec![input.consent_text().unwrap()];
        s.max_bytes = MAX_WIRE;
        s
    };
    let remote_scopes = [clip_scope(p.config.clone())];
    let local_scopes = [clip_scope(local.config().clone())];
    let remote = providers::ClipAttempt {
        provider: &p,
        scopes: &remote_scopes,
    };
    let settings = ProviderSelection::new(Operation::SuggestClips, p.config.clone());
    assert!(matches!(
        providers::suggest_clips(
            &settings,
            &[remote],
            &input,
            &mut approve(&log),
            &Hooks::default()
        ),
        Err(EditError::Ai(Error::Network))
    ));
    let settings = settings
        .with_fallbacks(vec![local.config().clone()])
        .unwrap();
    let result = providers::suggest_clips(
        &settings,
        &[
            providers::ClipAttempt {
                provider: &p,
                scopes: &remote_scopes,
            },
            providers::ClipAttempt {
                provider: &local,
                scopes: &local_scopes,
            },
        ],
        &input,
        &mut approve(&log),
        &Hooks::default(),
    )
    .unwrap();
    assert_eq!(result.provider, *local.config());
    assert_eq!(result.output.clips.len(), 1);
    assert_eq!(result.output.clips[0].source, ClipSource::Heuristic);
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire["provider"]["id"], "heuristic");
}
#[test]
fn success_exhaustion_and_cancelled_hooks_stop_at_the_right_provider() {
    for error in ["", "quota"] {
        let log = Arc::default();
        let p = fake("primary", error, &log);
        let f = fake("fallback", "auth", &log);
        let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
            .with_fallbacks(vec![f.config.clone()])
            .unwrap();
        let a = scope(p.config.clone());
        let b = scope(f.config.clone());
        let attempts = [
            TranslationAttempt {
                provider: &p,
                scope: &a,
            },
            TranslationAttempt {
                provider: &f,
                scope: &b,
            },
        ];
        let hooks = Hooks::default();
        hooks.0.store(true, Ordering::Relaxed);
        assert!(matches!(
            providers::translate(&settings, &attempts, &window(), &mut approve(&log), &hooks),
            Err(EditError::Ai(Error::Cancelled))
        ));
        assert!(log.lock().unwrap().is_empty());
        hooks.0.store(false, Ordering::Relaxed);
        let result =
            providers::translate(&settings, &attempts, &window(), &mut approve(&log), &hooks);
        if error.is_empty() {
            assert_eq!(result.unwrap().provider, p.config);
            assert_eq!(*log.lock().unwrap(), ["approve:primary:false", "primary"]);
        } else {
            assert!(matches!(result, Err(EditError::Ai(Error::Auth))));
            assert_eq!(
                *log.lock().unwrap(),
                [
                    "approve:primary:false",
                    "primary",
                    "approve:fallback:true",
                    "fallback"
                ]
            );
        }
    }
}

#[test]
fn job_result_bytes_record_the_actual_successful_provider() {
    use engine::{
        ByteBudget,
        cache::{ArtifactKind, CacheKey, KeyInputs},
        jobs::*,
    };
    use std::time::{Duration, Instant};
    let log = Arc::default();
    let p = fake("primary", "quota", &log);
    let f = fake("fallback", "", &log);
    let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
        .with_fallbacks(vec![f.config.clone()])
        .unwrap();
    let persisted = serde_json::to_vec(&settings).unwrap();
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Translation,
        source: [0; 32],
        provider: "configured-order",
        model: "fixture",
        version: "1",
        settings: &persisted,
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(65536).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    let (id, _) = runner
        .submit(
            JobSpec {
                kind: JobKind::Translate,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes {
                    result: 16384,
                    ..Default::default()
                },
            },
            move |hooks| {
                let a = scope(p.config.clone());
                let b = scope(f.config.clone());
                let attempts = [
                    TranslationAttempt {
                        provider: &p,
                        scope: &a,
                    },
                    TranslationAttempt {
                        provider: &f,
                        scope: &b,
                    },
                ];
                let result = providers::translate(
                    &settings,
                    &attempts,
                    &window(),
                    &mut approve(&log),
                    hooks,
                )?;
                serde_json::to_vec(&result)
                    .map_err(|_| EditError::Invalid("fixture result serialization"))
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        runner.tick().unwrap();
        let snapshot = runner.snapshot(id).unwrap();
        if snapshot.state == JobState::Completed {
            break;
        }
        assert!(!matches!(
            snapshot.state,
            JobState::Failed | JobState::Cancelled
        ));
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let output = runner.take_output(id).unwrap();
    let result: providers::ProviderOutput<Translation> =
        serde_json::from_slice(output.bytes()).unwrap();
    assert_eq!(result.provider.id().as_str(), "fallback");
    assert_eq!(result.output.texts[&NfcText::from("s1")].as_str(), "Hello");
}

#[test]
fn approval_failure_and_insufficient_cost_never_become_provider_failures() {
    struct FailedApproval;
    impl Approval for FailedApproval {
        fn approve(
            &mut self,
            _: &Scope,
            _: &CostEstimate,
            _: bool,
        ) -> Result<Option<Consent>, Error> {
            Err(Error::Auth)
        }
    }
    let log = Arc::default();
    let p = fake("primary", "", &log);
    let f = fake("fallback", "", &log);
    let settings = ProviderSelection::new(Operation::Translate, p.config.clone())
        .with_fallbacks(vec![f.config.clone()])
        .unwrap();
    let a = scope(p.config.clone());
    let b = scope(f.config.clone());
    let attempts = [
        TranslationAttempt {
            provider: &p,
            scope: &a,
        },
        TranslationAttempt {
            provider: &f,
            scope: &b,
        },
    ];
    assert!(matches!(
        providers::translate(
            &settings,
            &attempts,
            &window(),
            &mut FailedApproval,
            &Hooks::default()
        ),
        Err(EditError::Ai(Error::Auth))
    ));
    assert!(log.lock().unwrap().is_empty());
    let mut approval = approve(&log);
    approval.budget = 9;
    assert!(matches!(
        providers::translate(
            &settings,
            &attempts,
            &window(),
            &mut approval,
            &Hooks::default()
        ),
        Err(EditError::Ai(Error::Cost))
    ));
    assert_eq!(*log.lock().unwrap(), ["approve:primary:false"]);
}

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "integration fixtures"
)]
use ai::{openai::*, *};
use captions::{Cue, CueText, CueTrack, LanguageTag};
use engine::{
    ByteBudget, Command, EditError, Session,
    jobs::*,
    translation::{self, TranslationRun, TranslationTarget},
};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{SourceRange, Tick};
fn lang(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "".into()).unwrap().into_data();
    data.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            id: media::AssetId(1),
            path: std::env::temp_dir().join("translation synthetic source"),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: vec![],
            },
        })
        .unwrap(),
    );
    let cues = (0..4)
        .map(|i| Cue {
            id: format!("cue-{i}").into(),
            range: SourceRange::new(Tick(i * 10), Tick(i * 10 + 10)).unwrap(),
            source_lang: lang("vi"),
            texts: [
                (
                    lang("vi"),
                    CueText {
                        text: format!("nguồn {i}").into(),
                        edited: false,
                        translation_ref: None,
                    },
                ),
                (
                    lang("en"),
                    CueText {
                        text: "user edit".into(),
                        edited: i == 1,
                        translation_ref: None,
                    },
                ),
            ]
            .into(),
        })
        .collect();
    data.timeline.tracks.push(Track {
        id: TrackId(0),
        name: "captions".into(),
        kind: TrackKind::Caption,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            id: ItemId(0),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick(0), Tick(40)).unwrap(),
            start: Tick(0),
            content: ItemParams::Caption(CaptionParams {
                cues: CueTrack { cues },
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
fn cues(s: &Session) -> &[Cue] {
    let ItemParams::Caption(p) = &s.project().data().timeline.tracks[0].items[0].content else {
        panic!("fixture")
    };
    &p.cues.cues
}
struct Transport {
    calls: AtomicUsize,
    fail_count: usize,
}
impl HttpTransport for Transport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        let wire: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let w: TranslationWindow = serde_json::from_str(wire["input"].as_str().unwrap()).unwrap();
        let entries = if call < self.fail_count {
            vec![]
        } else {
            w.segments
                .iter()
                .map(|s| json!({"id":s.id,"text":format!("translated {}",s.text.as_str())}))
                .collect()
        };
        let result = json!({"translations":entries});
        Ok(HttpResponse{status:200,headers:vec![],body:serde_json::to_vec(&json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":result.to_string()}]}]})).unwrap()})
    }
}
fn try_run(
    session: &Session,
    w: TranslationWindow,
    fail_count: usize,
    max_attempts: u8,
    budget: u64,
) -> Result<(Arc<TranslationRun>, Arc<Transport>), EditError> {
    let config = ProviderConfig::new(
        "openai".into(),
        TRANSLATION_MODEL.into(),
        TRANSLATION_SETTINGS.into(),
        Location::Remote {
            backend: "openai".into(),
            base_url: BASE_URL.into(),
            auth_ref: "test".into(),
        },
    )
    .unwrap();
    let scope = Scope {
        provider: config.clone(),
        operation: Operation::Translate,
        payload_fingerprint: "test".into(),
        mode: TRANSLATION_MODE.into(),
        texts: vec![w.consent_text().unwrap()],
        source_language: w.source_language.clone(),
        target_language: Some(w.target_language.clone()),
        source_samples: SampleRange::new(0, 1).unwrap(),
        sample_rate: 1,
        max_bytes: 65536,
    };
    let c = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    let transport = Arc::new(Transport {
        calls: AtomicUsize::new(0),
        fail_count,
    });
    let p = Arc::new(
        OpenAiTranslator::new(config, ApiKey::new("test").unwrap(), transport.clone()).unwrap(),
    );
    Ok((
        TranslationRun::new(
            TranslationTarget {
                session,
                item: ItemId(0),
            },
            p,
            w,
            scope,
            Some(c),
            max_attempts,
            Money::new("USD", budget).unwrap(),
        )?,
        transport,
    ))
}
fn run(
    session: &Session,
    w: TranslationWindow,
    fail_count: usize,
    max_attempts: u8,
    budget: u64,
) -> (Arc<TranslationRun>, Arc<Transport>) {
    try_run(session, w, fail_count, max_attempts, budget).unwrap()
}
fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 2,
            retained: 2,
            network_attempts: 3,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(16 * 1024 * 1024).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}
fn pump(r: &mut JobRunner, id: JobId) -> JobSnapshot {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        r.tick().unwrap();
        let s = r.snapshot(id).unwrap();
        if matches!(
            s.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return s;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn selected(s: &Session, force: bool) -> TranslationWindow {
    translation::window(
        s,
        ItemId(0),
        &["cue-1".into(), "cue-2".into()],
        lang("en"),
        vec![],
        "concise".into(),
        force,
    )
    .unwrap()
}
#[test]
fn selection_context_and_explicit_json_force_preserve_user_edits_and_unselected_cues() {
    let mut s = session();
    let w = selected(&s, false);
    assert_eq!(w.segments.len(), 1);
    assert_eq!(w.segments[0].id.as_str(), "cue-2");
    assert_eq!(w.before.len(), 2);
    assert_eq!(w.after.len(), 1);
    assert_eq!(selected(&s, true).segments.len(), 2);
    for force in [false, true] {
        s.execute("retranslate",json!({"item":0,"target_language":"en","texts":{"cue-1":"changed","cue-2":"changed"},"translation_ref":"openai/test","force":force})).unwrap();
        assert_eq!(
            cues(&s)[1].texts[&lang("en")].text.as_str(),
            if force { "changed" } else { "user edit" }
        );
        assert_eq!(cues(&s)[0].texts[&lang("en")].text.as_str(), "user edit");
        assert_eq!(cues(&s)[3].texts[&lang("en")].text.as_str(), "user edit");
    }
    let before = s.project().clone();
    assert!(s.execute("retranslate",json!({"item":0,"target_language":"en","texts":{"cue-1":"changed","missing":"bad"},"translation_ref":"test","force":true})).is_err());
    assert!(Arc::ptr_eq(s.project(), &before));
}
#[test]
fn engine_repairs_schema_with_same_provider_then_applies_one_revision_bound_command() {
    let mut s = session();
    let (run, t) = run(&s, selected(&s, false), 1, 2, 1_000_000);
    let mut r = runner();
    let (id, _) = run.submit(&mut r, &s).unwrap();
    let done = pump(&mut r, id);
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.attempts, 2);
    assert_eq!(t.calls.load(Ordering::Relaxed), 2);
    let output = r.take_output(id).unwrap();
    let translated: Translation = serde_json::from_slice(output.bytes()).unwrap();
    output
        .apply(
            &mut s,
            Command::Retranslate {
                item: ItemId(0),
                target_language: translated.target_language,
                texts: translated.texts,
                translation_ref: "openai/test".into(),
                force: false,
            },
        )
        .unwrap();
    assert_eq!(cues(&s)[1].texts[&lang("en")].text.as_str(), "user edit");
    assert!(
        cues(&s)[2].texts[&lang("en")]
            .text
            .as_str()
            .starts_with("translated")
    );
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    assert_eq!(cues(&s)[2].texts[&lang("en")].text.as_str(), "user edit");
}
#[test]
fn retries_cannot_reset_operation_budget_and_late_results_refuse_changed_session() {
    let mut s = session();
    let (run, t) = run(&s, selected(&s, false), 10, 1, 1_000_000);
    let mut r = runner();
    let (id, _) = run.submit(&mut r, &s).unwrap();
    let done = pump(&mut r, id);
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    assert!(r.take_output(id).is_none());
    let (run, _) = self::run(&s, selected(&s, false), 0, 1, 1_000_000);
    let (id, _) = run.submit(&mut r, &s).unwrap();
    s.apply(Command::AddTrack {
        name: "".into(),
        kind: TrackKind::Overlay,
    })
    .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    let output = r.take_output(id).unwrap();
    let result: Translation = serde_json::from_slice(output.bytes()).unwrap();
    assert!(matches!(
        output.apply(
            &mut s,
            Command::Retranslate {
                item: ItemId(0),
                target_language: result.target_language,
                texts: result.texts,
                translation_ref: "test".into(),
                force: false
            }
        ),
        Err(EditError::StaleResult)
    ));
}

#[test]
fn operation_budget_covers_all_attempts_before_any_translation_starts() {
    let s = session();
    assert!(matches!(
        try_run(&s, selected(&s, false), 0, 2, 344064),
        Err(EditError::Ai(Error::Cost))
    ));
    assert!(matches!(
        try_run(&s, selected(&s, false), 0, 0, 1_000_000),
        Err(EditError::Limit)
    ));
}

#[test]
fn consent_preparation_cannot_rebind_old_source_to_a_new_session_revision() {
    let mut s = session();
    let mut stale = selected(&s, false);
    stale.segments.first_mut().unwrap().text = "old source".into();
    assert!(matches!(
        try_run(&s, stale, 0, 1, 1_000_000),
        Err(EditError::StaleResult)
    ));
    let (run, transport) = run(&s, selected(&s, false), 0, 1, 1_000_000);
    s.apply(Command::AddTrack {
        name: "edit during consent".into(),
        kind: TrackKind::Overlay,
    })
    .unwrap();
    assert!(matches!(
        run.submit(&mut runner(), &s),
        Err(EditError::StaleResult)
    ));
    assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
}

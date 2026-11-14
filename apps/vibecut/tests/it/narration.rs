#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
//! The Text to speech tab through a scripted provider: no network, no key, no native app.
use ai::speech::{
    SpeechAudio, SpeechCapabilities, SpeechFormat, SpeechLanguage, SpeechOutput, SpeechPolicy,
    SpeechRequest, SpeechSynthesizer, SpeechUsage, SpeechVoice, SynthesisConsent,
    SynthesisEstimate, VoiceCategory, prepare_synthesis,
};
use ai::{ExecutionHooks, Location, Money, ProviderConfig, Support};
use captions::LanguageTag;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::dubbing::SpeechOpener;
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session};
use media::{AssetId, ContentHash, SourceAsset};
use project::{
    AnimationPath, AudioSource, BuiltinPath, ItemId, ItemParams, Project, ProjectId, TrackId,
    TrackKind,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, egui,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;
const RATE: u32 = 24_000;
const POLICY: &str = "https://example.com/policy";

/// How the scripted provider answers one synthesis.
#[derive(Clone, Copy)]
enum Script {
    /// This many samples of a quiet tone.
    Samples(usize),
    Fail(fn() -> ai::Error),
}

/// One scripted voice: the same adapter contract a real provider implements, with no wire.
struct Voice {
    config: ProviderConfig,
    script: Script,
    calls: AtomicUsize,
}
impl Voice {
    fn new(provider: &str, model: &str, script: Script) -> Arc<Self> {
        Arc::new(Self {
            config: ProviderConfig::new(
                provider.into(),
                model.into(),
                "settings".into(),
                Location::Remote {
                    backend: "fixture".into(),
                    base_url: "https://example.com/v1".into(),
                    auth_ref: "secret-reference".into(),
                },
            )
            .unwrap(),
            script,
            calls: AtomicUsize::new(0),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn caps(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            provider: self.config.clone(),
            fingerprint: format!(
                "{}:{}",
                self.config.id().as_str(),
                self.config.model().as_str()
            )
            .as_str()
            .into(),
            languages: ["vi", "en"]
                .iter()
                .map(|tag| SpeechLanguage {
                    language: LanguageTag::parse(tag).unwrap(),
                    script: "Latn".into(),
                })
                .collect(),
            voices: ["alloy", "21m00Tcm4TlvDq8ikWAM", "EXAVITQu4vr4xnSDxMaL"]
                .iter()
                .map(|id| SpeechVoice {
                    id: (*id).into(),
                    category: VoiceCategory::BuiltIn,
                })
                .collect(),
            outputs: vec![output()],
            max_characters: Some(500),
            streaming: Support::Unsupported,
            max_concurrency: Some(1),
            requests_per_minute: None,
            cost_model: "bounded USD invocation".into(),
            policy: SpeechPolicy {
                policy_url: POLICY.into(),
                retention: "kept for 30 days".into(),
                training: "not used for training".into(),
                voice_disclosure: "synthetic voice".into(),
            },
        }
    }
}
fn output() -> SpeechOutput {
    SpeechOutput {
        format: SpeechFormat::Pcm16,
        sample_rate: RATE,
    }
}
impl SpeechSynthesizer for Voice {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &SpeechRequest) -> SpeechCapabilities {
        self.caps()
    }
    fn estimate(&self, _: &SpeechRequest) -> SynthesisEstimate {
        SynthesisEstimate::UpperBound(Money::new("USD", 2_500).unwrap())
    }
    fn synthesize(
        &self,
        request: &SpeechRequest,
        consent: &SynthesisConsent,
        _: &dyn ExecutionHooks,
    ) -> Result<SpeechAudio, ai::Error> {
        prepare_synthesis(self, request, consent)?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let count = match self.script {
            Script::Fail(error) => return Err(error()),
            Script::Samples(count) => count,
        };
        Ok(SpeechAudio {
            samples: vec![0.25; count],
            sample_rate: RATE,
            duration_micros: (count as u64 * 1_000_000).div_ceil(u64::from(RATE)),
            fingerprint: self.caps().fingerprint,
            usage: SpeechUsage {
                billable_characters: Some(request.characters()),
                input_tokens: None,
                output_tokens: None,
                actual_cost: Some(Money::new("USD", 1_000).unwrap()),
            },
            transcript: Some(request.text().clone()),
        })
    }
}

/// Opens only the two provider and model pairs the panel lists by default.
struct Opener {
    openai: Arc<Voice>,
    elevenlabs: Arc<Voice>,
    attempts: Mutex<Vec<(String, String, Option<u16>)>>,
}
impl SpeechOpener for Opener {
    fn open(
        &self,
        provider: &str,
        model: &str,
        speed: Option<u16>,
    ) -> Result<(Arc<dyn SpeechSynthesizer>, SpeechOutput), ai::Error> {
        self.attempts
            .lock()
            .unwrap()
            .push((provider.to_owned(), model.to_owned(), speed));
        let voice: Arc<dyn SpeechSynthesizer> = match (provider, model) {
            ("openai", "gpt-audio-1.5") => self.openai.clone(),
            ("elevenlabs", "eleven_flash_v2_5") => self.elevenlabs.clone(),
            _ => return Err(ai::Error::Unsupported),
        };
        Ok((voice, output()))
    }
}

struct Bench {
    harness: Harness<'static, Editor>,
    text: ItemId,
    audio: ItemId,
    opener: Arc<Opener>,
    directory: tempfile::TempDir,
}

fn audio_asset(id: u64) -> SourceAsset {
    let mut stream = testkit::source::stream(media::StreamKind::Audio)
        .duration(Tick(10 * SECOND))
        .build();
    stream.index = 0;
    testkit::source::asset(
        id,
        std::env::temp_dir().join(format!("vibecut-narration-{id}.m4a")),
    )
    .format("mov,mp4")
    .duration(Tick(10 * SECOND))
    .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
    .streams(vec![stream])
    .build()
}

fn insert(
    session: &mut Session,
    track: TrackId,
    end: Tick,
    asset: Option<AssetId>,
    content: ItemParams,
) -> ItemId {
    let outcome = session
        .apply(Command::InsertItem {
            placement: Placement::Track { track },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: asset,
                source_range: SourceRange::new(Tick::ZERO, end).unwrap(),
                start: Tick::ZERO,
                content,
            },
        })
        .unwrap();
    match outcome.created.first().copied() {
        Some(CreatedId::Item(id)) => id,
        _ => panic!("item"),
    }
}

fn track(session: &mut Session, name: &str, kind: TrackKind) -> TrackId {
    let outcome = session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: name.into(),
            kind,
        })
        .unwrap();
    match outcome.created.first().copied() {
        Some(CreatedId::Track(id)) => id,
        _ => panic!("track"),
    }
}

/// A two-second text cue over a two-second original audio item, and a tall window so every
/// row of the tab is on screen.
fn bench(openai: Script, elevenlabs: Script) -> Bench {
    let mut session = Session::new(
        Project::empty(ProjectId(7), "Narration".into()).unwrap(),
        50,
    )
    .unwrap();
    session
        .apply(Command::AddAsset {
            asset: audio_asset(2),
        })
        .unwrap();
    let text_track = track(&mut session, "Text", TrackKind::Text);
    let audio_track = track(&mut session, "Audio", TrackKind::Audio);
    let text = insert(
        &mut session,
        text_track,
        Tick(2 * SECOND),
        None,
        ItemParams::Text(project::TextParams {
            content: "Welcome back.".into(),
            font_size: 48.0,
            ..Default::default()
        }),
    );
    let audio = insert(
        &mut session,
        audio_track,
        Tick(2 * SECOND),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams::default()),
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    let opener = Arc::new(Opener {
        openai: Voice::new("openai", "gpt-audio-1.5", openai),
        elevenlabs: Voice::new("elevenlabs", "eleven_flash_v2_5", elevenlabs),
        attempts: Mutex::new(Vec::new()),
    });
    let directory = tempfile::tempdir().unwrap();
    editor.set_narration(opener.clone(), directory.path().to_path_buf());
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 2200.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    harness
        .state_mut()
        .select([text].into_iter().collect())
        .unwrap();
    harness.run_steps(3);
    let mut bench = Bench {
        harness,
        text,
        audio,
        opener,
        directory,
    };
    click(&mut bench.harness, "inspector.tab.text_to_speech");
    bench
}

fn has_widget(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .query(By::new().predicate(|node| node.author_id() == Some(id)))
        .unwrap_or_else(|| panic!("missing {id}"))
        .click();
    harness.run_steps(3);
}

/// Pick one dropdown option by the automation id its row registers.
fn choose(harness: &mut Harness<Editor>, dropdown: &str, index: usize) {
    click(harness, dropdown);
    click(harness, &format!("{dropdown}.{index}"));
}

/// Step the frame until a widget appears; the worker thread finishes on its own time.
fn wait_for(harness: &mut Harness<Editor>, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !has_widget(harness, id) {
        assert!(Instant::now() < deadline, "{id} never appeared");
        std::thread::sleep(Duration::from_millis(2));
        harness.run_steps(1);
    }
}

fn narrations(harness: &Harness<Editor>) -> Vec<(ItemId, project::Narration)> {
    harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .filter_map(|item| match &item.content {
            ItemParams::Audio(params) => match &params.source {
                AudioSource::Narration { narration } => Some((item.id, narration.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn ducked(harness: &Harness<Editor>, item: ItemId) -> bool {
    let path = AnimationPath::from(BuiltinPath::Volume);
    harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .is_some_and(|found| found.animations.get(&path).is_some())
}

fn save(harness: &mut Harness<Editor>, name: &str) {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    harness.render().unwrap().save(target.join(name)).unwrap();
}

#[test]
fn the_chosen_provider_and_voice_generate_consent_fit_place_and_duck_in_one_undo_step() {
    // 2.1 s of audio for a 2 s cue: a 1.050x stretch.
    let mut bench = bench(Script::Samples(72_000), Script::Samples(50_400));
    let h = &mut bench.harness;
    for id in [
        "inspector.narration.provider",
        "inspector.narration.model",
        "inspector.narration.voice",
        "inspector.narration.language",
        "inspector.narration.text",
        "inspector.narration.generate",
    ] {
        assert!(has_widget(h, id), "{id}");
    }
    // Nothing is chosen for the person: pick ElevenLabs and its second premade voice.
    choose(h, "inspector.narration.provider", 1);
    choose(h, "inspector.narration.voice", 1);
    let history = h.state().session.history_len().0;

    // Declining the sheet sends nothing and changes nothing.
    click(h, "inspector.narration.generate");
    wait_for(h, "narration.consent.send");
    assert!(has_widget(h, "narration.consent.deny"));
    assert_eq!(
        bench.opener.elevenlabs.calls() + bench.opener.openai.calls(),
        0
    );
    click(h, "narration.consent.deny");
    wait_for(h, "inspector.narration.failure");
    assert!(!has_widget(h, "inspector.narration.take"));
    assert_eq!(bench.opener.elevenlabs.calls(), 0);
    assert!(narrations(h).is_empty());
    assert_eq!(h.state().session.history_len().0, history);

    // Approving runs one attempt on the chosen provider only: the other is never opened.
    click(h, "inspector.narration.generate");
    wait_for(h, "narration.consent.send");
    click(h, "narration.consent.send");
    wait_for(h, "inspector.narration.take");
    assert_eq!(bench.opener.elevenlabs.calls(), 1);
    assert_eq!(bench.opener.openai.calls(), 0);
    assert!(
        bench
            .opener
            .attempts
            .lock()
            .unwrap()
            .iter()
            .all(
                |(provider, model, speed)| (provider.as_str(), model.as_str(), *speed)
                    == ("elevenlabs", "eleven_flash_v2_5", None)
            )
    );

    // The fit is confirmed before anything is placed: a stretch, with Place offered.
    assert!(has_widget(h, "inspector.narration.fit"));
    assert!(has_widget(h, "inspector.narration.place"));
    assert!(narrations(h).is_empty());
    assert_eq!(h.state().session.history_len().0, history);
    // Ducking is chosen on the pending take: switch it off before placing.
    for id in [
        "inspector.narration.ducking.enabled",
        "inspector.narration.ducking.depth",
        "inspector.narration.ducking.attack",
        "inspector.narration.ducking.release",
    ] {
        assert!(has_widget(h, id), "{id}");
    }
    click(h, "inspector.narration.ducking.enabled");
    save(h, "p2-dubbing-02-tts.png");

    // Place is one undo step: asset, item, stretch and the (disabled) ducking together.
    click(h, "inspector.narration.place");
    let placed = narrations(h);
    assert_eq!(placed.len(), 1);
    let (item, record) = placed[0].clone();
    assert_eq!(h.state().session.history_len().0, history + 1);
    assert_eq!(
        (
            record.provider.as_str(),
            record.model.as_str(),
            record.voice.as_str()
        ),
        ("elevenlabs", "eleven_flash_v2_5", "EXAVITQu4vr4xnSDxMaL")
    );
    assert!(!record.ducking.enabled);
    assert_eq!(record.cue_duration, Tick(2 * SECOND));
    assert!(!ducked(h, bench.audio));
    assert!(has_widget(h, "inspector.narration.ducking.enabled"));
    assert!(!has_widget(h, "inspector.narration.place"));
    let written = std::fs::read_dir(bench.directory.path()).unwrap().count();
    assert_eq!(written, 1, "one take file");

    // After placement the same switch is `dub.set_ducking`: the original audio now ducks.
    click(h, "inspector.narration.ducking.enabled");
    assert_eq!(h.state().session.history_len().0, history + 2);
    assert!(narrations(h)[0].1.ducking.enabled);
    assert!(ducked(h, bench.audio));

    // One undo removes the ducking, one more the whole placement.
    h.state_mut().session.undo().unwrap();
    assert!(!ducked(h, bench.audio));
    h.state_mut().session.undo().unwrap();
    h.run_steps(2);
    assert!(narrations(h).is_empty());
    assert_eq!(h.state().session.history_len().0, history);
    let _ = (item, bench.text);
}

#[test]
fn an_unsupported_language_is_refused_before_consent_and_places_nothing() {
    let mut bench = bench(Script::Samples(48_000), Script::Samples(48_000));
    let h = &mut bench.harness;
    let before = h.state().session.project().clone();
    // The scripted provider speaks Vietnamese and English only; Chinese is the third entry.
    choose(h, "inspector.narration.language", 2);
    click(h, "inspector.narration.generate");
    h.run_steps(3);
    assert!(has_widget(h, "inspector.narration.failure"));
    assert!(!has_widget(h, "narration.consent.send"));
    assert!(!has_widget(h, "inspector.narration.take"));
    assert_eq!(
        bench.opener.openai.calls() + bench.opener.elevenlabs.calls(),
        0
    );
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn a_provider_failure_discards_the_take_and_places_nothing() {
    let mut bench = bench(Script::Fail(|| ai::Error::Quota), Script::Samples(48_000));
    let h = &mut bench.harness;
    let before = h.state().session.project().clone();
    click(h, "inspector.narration.generate");
    wait_for(h, "narration.consent.send");
    click(h, "narration.consent.send");
    wait_for(h, "inspector.narration.failure");
    assert_eq!(bench.opener.openai.calls(), 1);
    assert!(!has_widget(h, "inspector.narration.take"));
    assert!(narrations(h).is_empty());
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn a_take_too_long_for_its_cue_asks_for_a_shorter_text_and_places_nothing() {
    // 3 s of audio for a 2 s cue is 1.5x; OpenAI has no speed setting, so it asks to shorten.
    let mut bench = bench(Script::Samples(72_000), Script::Samples(48_000));
    let h = &mut bench.harness;
    let before = h.state().session.project().clone();
    click(h, "inspector.narration.generate");
    wait_for(h, "narration.consent.send");
    click(h, "narration.consent.send");
    wait_for(h, "inspector.narration.take");
    assert!(has_widget(h, "inspector.narration.fit"));
    assert!(!has_widget(h, "inspector.narration.place"));
    assert!(!has_widget(h, "inspector.narration.regenerate"));
    assert!(has_widget(h, "inspector.narration.discard"));
    click(h, "inspector.narration.discard");
    assert!(!has_widget(h, "inspector.narration.take"));
    assert_eq!(h.state().session.project(), &before);
    let _ = bench.text;
}

#[test]
fn closing_the_editor_while_consent_is_asked_stops_the_job_and_sends_nothing() {
    // The job waits in the consent sheet; dropping the editor must end that wait before the
    // runner joins the job, or the drop never returns.
    let mut bench = bench(Script::Samples(48_000), Script::Samples(48_000));
    let h = &mut bench.harness;
    click(h, "inspector.narration.generate");
    wait_for(h, "narration.consent.send");
    let opener = bench.opener.clone();
    drop(bench);
    assert_eq!(opener.openai.calls(), 0);
}

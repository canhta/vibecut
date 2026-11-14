#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
//! Narration: explicit provider selection, consent, duration fit, placement and ducking.
use crate::support::control::{seed, with_audio, with_narration};
use crate::support::dubbing::{self as fx, Opener, Script, Voice};
use ai::speech::SynthesisEstimate;
use captions::LanguageTag;
use engine::dubbing::{
    CancelFlag, DurationFit, Fit, FitInput, Narrator, Request, decode_take, encode_take, fit,
    ticks_of, wav_bytes,
};
use engine::providers::{ConsentPrompt, Cost, Error, UploadRequest};
use engine::{Command, CreatedId, EditError, NewItem, Placement, Session};
use project::{
    AnimationPath, AudioParams, AudioSource, BuiltinPath, Item, ItemId, ItemParams,
    NarrationDucking, ParamValue, TrackId,
};
use std::sync::{Arc, Mutex};
use time::{SourceRange, Tick};

const SECOND: i64 = time::TICKS_PER_SECOND;

// ----- fit tiers -----

fn input(natural: i64, cue: i64) -> FitInput {
    FitInput {
        natural: Tick(natural),
        cue: Tick(cue),
        silent_after: Tick(0),
        characters: 100,
        provider_speed: false,
        rephrased: false,
    }
}

#[test]
fn fit_tiers_stretch_within_the_limits_then_rephrase_then_overflow_or_fail() {
    let cue = 10 * SECOND;
    assert_eq!(fit(input(10 * SECOND, cue)), Ok(Fit::Exact));
    assert_eq!(
        fit(input(10 * SECOND + SECOND / 2, cue)),
        Ok(Fit::Stretch { thousandths: 1050 })
    );
    assert_eq!(
        fit(input(11 * SECOND, cue)),
        Ok(Fit::Stretch { thousandths: 1100 })
    );
    assert_eq!(
        fit(input(9 * SECOND, cue)),
        Ok(Fit::Stretch { thousandths: 900 })
    );
    // Too short to stretch: it plays as generated and leaves a gap.
    assert_eq!(fit(input(8 * SECOND, cue)), Ok(Fit::Gap));
    // Too long: one rephrase with a character budget (100 * 10 * 1.1 / 12 = 91).
    assert_eq!(
        fit(input(12 * SECOND, cue)),
        Ok(Fit::Rephrase { max_characters: 91 })
    );
    // A provider speed setting may be tried first, and it is shown as such.
    let mut speedy = input(12 * SECOND, cue);
    speedy.provider_speed = true;
    assert_eq!(fit(speedy), Ok(Fit::ProviderSpeed { thousandths: 1091 }));
    // Beyond what the provider speed can recover (1.2 * 1.1) a rephrase is asked.
    let mut hopeless = input(14 * SECOND, cue);
    hopeless.provider_speed = true;
    assert!(matches!(fit(hopeless), Ok(Fit::Rephrase { .. })));
    // After the rephrase, overflow needs silence after the cue and at most 1.5x.
    let mut rephrased = input(12 * SECOND, cue);
    rephrased.rephrased = true;
    rephrased.silent_after = Tick(3 * SECOND);
    let over = i64::try_from(i128::from(12 * SECOND) * 1000 / 1100 - i128::from(cue)).unwrap();
    assert_eq!(
        fit(rephrased),
        Ok(Fit::Overflow {
            thousandths: 1100,
            over: Tick(over)
        })
    );
    rephrased.silent_after = Tick(SECOND / 2);
    assert_eq!(fit(rephrased), Err(DurationFit));
    let mut too_long = input(17 * SECOND, cue);
    too_long.rephrased = true;
    too_long.silent_after = Tick(10 * SECOND);
    assert_eq!(fit(too_long), Err(DurationFit));
    assert_eq!(fit(input(0, cue)), Err(DurationFit));
    assert_eq!(fit(input(SECOND, 0)), Err(DurationFit));
}

// ----- provider selection, consent and the synthesis seam -----

struct Prompt {
    seen: Mutex<Vec<UploadRequest>>,
    allow: bool,
    allow_unknown: bool,
}
impl Prompt {
    fn new(allow: bool, allow_unknown: bool) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            allow,
            allow_unknown,
        }
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}
impl ConsentPrompt for Prompt {
    fn approve(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        self.seen.lock().unwrap().push(request.clone());
        if self.allow {
            Ok(())
        } else {
            Err(Error::ConsentRequired)
        }
    }
    fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        self.seen.lock().unwrap().push(request.clone());
        if self.allow_unknown {
            Ok(())
        } else {
            Err(ai::Error::Cost.into())
        }
    }
}

fn request(model: &str, text: &str) -> Request {
    Request {
        provider: "scripted".into(),
        model: model.into(),
        voice: "alloy".into(),
        language: LanguageTag::parse("en").unwrap(),
        text: text.into(),
        cue_duration: Tick(SECOND),
        provider_speed_thousandths: None,
    }
}
fn narrator(voices: Vec<(&str, Arc<Voice>)>) -> (Narrator, Arc<Opener>) {
    let opener = Arc::new(Opener::new(
        voices
            .into_iter()
            .map(|(model, voice)| ("scripted", model, voice))
            .collect(),
    ));
    (Narrator::new(opener.clone()), opener)
}
fn voice(script: Script) -> Arc<Voice> {
    Arc::new(Voice::new("scripted", "model-a", script))
}

#[test]
fn provider_model_and_voice_are_explicit_and_never_fall_back() {
    let a = voice(Script::Samples(24_000));
    let b = Arc::new(Voice::new("scripted", "model-b", Script::Samples(24_000)));
    let (narrator, opener) = narrator(vec![("model-a", a.clone()), ("model-b", b.clone())]);
    let prompt = Prompt::new(true, false);

    // An unknown model is refused outright: no other model is tried, nothing is asked.
    let refused = narrator.prepare(&request("model-c", "Hello there."));
    assert!(matches!(refused, Err(Error::Ai(ai::Error::Unsupported))));
    assert_eq!(
        *opener.attempts.lock().unwrap(),
        [("scripted".to_owned(), "model-c".to_owned(), None)]
    );
    assert_eq!((a.calls(), b.calls(), prompt.count()), (0, 0, 0));

    // The chosen model is the one that is disclosed, approved and called.
    let prepared = narrator
        .prepare(&request("model-b", "Hello there."))
        .unwrap();
    assert_eq!(prepared.disclosure().model, "model-b");
    let consent = prepared.approve(&prompt).unwrap();
    prepared.synthesize(&consent, &CancelFlag::new()).unwrap();
    assert_eq!((a.calls(), b.calls(), prompt.count()), (0, 1, 1));
}

#[test]
fn unsupported_language_and_voice_are_typed_and_come_before_consent() {
    let v = voice(Script::Samples(24_000));
    let (narrator, _) = narrator(vec![("model-a", v.clone())]);
    let mut chinese = request("model-a", "你好");
    chinese.language = LanguageTag::parse("zh").unwrap();
    assert!(matches!(
        narrator.prepare(&chinese),
        Err(Error::Ai(ai::Error::UnsupportedLanguage))
    ));
    let mut french = request("model-a", "Bonjour");
    french.language = LanguageTag::parse("fr").unwrap();
    assert!(matches!(
        narrator.prepare(&french),
        Err(Error::Ai(ai::Error::UnsupportedLanguage))
    ));
    let mut stranger = request("model-a", "Hello");
    stranger.voice = "cloned-voice".into();
    assert!(matches!(
        narrator.prepare(&stranger),
        Err(Error::Ai(ai::Error::UnsupportedVoice))
    ));
    assert_eq!(v.calls(), 0);
}

#[test]
fn consent_shows_the_exact_payload_and_binds_the_quote_and_text() {
    let v = voice(Script::Samples(24_000));
    let (narrator, _) = narrator(vec![("model-a", v.clone())]);
    let mut speedy = request(
        "model-a",
        "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t is spoken.",
    );
    speedy.language = LanguageTag::parse("vi").unwrap();
    speedy.provider_speed_thousandths = Some(1100);
    let prepared = narrator.prepare(&speedy).unwrap();
    let shown = prepared.disclosure();
    assert_eq!(
        (
            shown.provider.as_str(),
            shown.model.as_str(),
            shown.voice.as_str()
        ),
        ("scripted", "model-a", "alloy")
    );
    assert_eq!(
        (shown.language.as_str(), shown.script.as_str()),
        ("vi", "Latn")
    );
    assert_eq!(shown.characters, 21);
    assert!(shown.preview.starts_with("Tiếng Việt"));
    assert_eq!(shown.text_sha256.len(), 64);
    assert_eq!(shown.provider_speed_thousandths, Some(1100));
    assert_eq!(shown.policy_url, fx::POLICY);
    assert_eq!(shown.retention, "kept for 30 days");
    assert_eq!(shown.training, "not used for training");
    assert_eq!(shown.voice_disclosure, "synthetic voice");
    assert_eq!(
        shown.cost,
        Cost::UpperBound {
            currency: "USD".into(),
            micros: 2_500
        }
    );
    // The same facts reach the shared consent prompt.
    let prompt = Prompt::new(true, false);
    let consent = prepared.approve(&prompt).unwrap();
    let asked = prompt.seen.lock().unwrap()[0].clone();
    let term = |name: &str| {
        asked
            .terms
            .iter()
            .find(|(label, _)| *label == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(term("voice_disclosure").as_deref(), Some("synthetic voice"));
    assert_eq!(term("retention").as_deref(), Some("kept for 30 days"));
    assert_eq!(term("training").as_deref(), Some("not used for training"));
    assert_eq!(term("characters").as_deref(), Some("21"));
    assert_eq!(term("speed").as_deref(), Some("1.1x"));
    assert_eq!(asked.model, "model-a");

    // A token for one text cannot run another.
    let other = narrator
        .prepare(&request("model-a", "A different line."))
        .unwrap();
    assert!(matches!(
        other.synthesize(&consent, &CancelFlag::new()),
        Err(ai::Error::ConsentRequired)
    ));
    assert_eq!(v.calls(), 0);

    // A refusal never reaches the provider.
    let denied = narrator
        .prepare(&request("model-a", "Hello there."))
        .unwrap();
    assert!(matches!(
        denied.approve(&Prompt::new(false, false)),
        Err(Error::ConsentRequired)
    ));
    assert_eq!(v.calls(), 0);
}

#[test]
fn an_unknown_price_needs_its_own_acknowledgement() {
    let unknown = Arc::new(
        Voice::new("scripted", "model-a", Script::Samples(24_000))
            .priced(SynthesisEstimate::Unknown),
    );
    let (narrator, _) = narrator(vec![("model-a", unknown.clone())]);
    let prepared = narrator
        .prepare(&request("model-a", "Hello there."))
        .unwrap();
    assert_eq!(prepared.disclosure().cost, Cost::Unknown);
    assert!(matches!(
        prepared.approve(&Prompt::new(true, false)),
        Err(Error::Ai(ai::Error::Cost))
    ));
    let consent = prepared.approve(&Prompt::new(true, true)).unwrap();
    let take = prepared.synthesize(&consent, &CancelFlag::new()).unwrap();
    assert_eq!(take.narration.consent.upper_bound, None);

    let (cheap, _) = narrator_over_budget();
    assert!(matches!(
        cheap.prepare(&request("model-a", &"x".repeat(501))),
        Err(Error::Ai(ai::Error::Payload))
    ));
}
fn narrator_over_budget() -> (Narrator, Arc<Opener>) {
    narrator(vec![("model-a", voice(Script::Samples(24_000)))])
}

#[test]
fn a_take_carries_its_provenance_and_exact_duration() {
    let v = voice(Script::Samples(36_000));
    let (narrator, _) = narrator(vec![("model-a", v)]);
    let prepared = narrator
        .prepare(&request("model-a", "Hello there."))
        .unwrap();
    let consent = prepared.approve(&Prompt::new(true, false)).unwrap();
    let take = prepared.synthesize(&consent, &CancelFlag::new()).unwrap();
    assert_eq!(take.sample_rate, 24_000);
    assert_eq!(take.pcm.len(), 36_000);
    assert!(take.pcm.iter().all(|sample| *sample == 8_192));
    // 36 000 samples at 24 kHz are exactly 1.5 s of timeline.
    assert_eq!(take.natural().unwrap(), Tick(SECOND * 3 / 2));
    // One sample is 1/24000 s: 10 584 000 ticks, exact.
    assert_eq!(ticks_of(1, 24_000).unwrap(), Tick(10_584_000));
    let wav = wav_bytes(take.sample_rate, &take.pcm);
    assert_eq!(wav.len(), 44 + 72_000);
    assert_eq!(&wav[..4], b"RIFF");
    let n = &take.narration;
    assert_eq!(
        (n.provider.as_str(), n.model.as_str(), n.voice.as_str()),
        ("scripted", "model-a", "alloy")
    );
    assert_eq!(n.fingerprint.as_str(), "scripted:model-a");
    assert_eq!(n.characters, 12);
    assert_eq!(n.consent.upper_bound.as_ref().unwrap().micros, 2_500);
    assert_eq!(n.cue_duration, Tick(SECOND));
    assert_eq!(n.ducking, NarrationDucking::DEFAULT);
    assert_eq!(decode_take(&encode_take(&take).unwrap()).unwrap(), take);
}

#[test]
fn failures_rejections_and_cancel_leave_no_take() {
    for (script, check) in [
        (
            Script::Fail(|| ai::Error::Server),
            (|e: &ai::Error| matches!(e, ai::Error::Server)) as fn(&ai::Error) -> bool,
        ),
        (Script::Fail(|| ai::Error::Quota), |e| {
            matches!(e, ai::Error::Quota)
        }),
        (Script::WrongTranscript, |e| {
            matches!(e, ai::Error::OutputRejected)
        }),
    ] {
        let (narrator, _) = narrator(vec![("model-a", voice(script))]);
        let prepared = narrator
            .prepare(&request("model-a", "Hello there."))
            .unwrap();
        let consent = prepared.approve(&Prompt::new(true, false)).unwrap();
        let error = prepared
            .synthesize(&consent, &CancelFlag::new())
            .unwrap_err();
        assert!(check(&error), "{error:?}");
    }
    let v = voice(Script::Samples(24_000));
    let (narrator, _) = narrator(vec![("model-a", v.clone())]);
    let prepared = narrator
        .prepare(&request("model-a", "Hello there."))
        .unwrap();
    let consent = prepared.approve(&Prompt::new(true, false)).unwrap();
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert!(matches!(
        prepared.synthesize(&consent, &cancel),
        Err(ai::Error::Cancelled)
    ));
    assert_eq!(v.calls(), 0);
}

// ----- placement, fit edits, ducking and history -----

fn narrations(s: &Session) -> Vec<Item> {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter().cloned())
        .filter(|item| {
            matches!(
                &item.content,
                ItemParams::Audio(AudioParams {
                    source: AudioSource::Narration { .. },
                    ..
                })
            )
        })
        .collect()
}
fn item(s: &Session, id: ItemId) -> Item {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .cloned()
        .unwrap()
}
/// The volume (dB) of an item at `at` ticks after its start.
fn level(s: &Session, id: ItemId, at: i64) -> f64 {
    let item = item(s, id);
    let path = AnimationPath::from(BuiltinPath::Volume);
    match item
        .animations
        .get(&path)
        .map(|param| param.value_at(Tick(at)))
    {
        Some(Ok(ParamValue::Scalar(value))) => value,
        None => 0.0,
        other => panic!("{other:?}"),
    }
}
fn place(s: &mut Session, asset: u64, start: i64, duration: i64, cue: i64, speed: f64) -> ItemId {
    let outcome = s
        .apply(Command::DubPlace {
            placement: Placement::Auto,
            start: Tick(start),
            asset: fx::asset(asset, Tick(duration)),
            narration: fx::narration(Tick(cue)),
            speed: time::Speed::from_f64(speed).unwrap(),
        })
        .unwrap();
    outcome
        .created
        .iter()
        .find_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .unwrap()
}
/// Only the original clip 5 and the narration 6 remain, so silence is known.
fn isolated() -> Session {
    let base = with_narration(&seed());
    let mut data = base.project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        track
            .items
            .retain(|item| item.id == ItemId(5) || item.id == ItemId(6));
    }
    Session::new(project::Project::new(data).unwrap(), 10).unwrap()
}

#[test]
fn a_repeated_cue_places_each_take_ducks_the_original_and_undoes_as_one_step() {
    let mut s = with_audio(&seed());
    let start = project::encode(s.project(), false).unwrap();
    let a = place(&mut s, 201, 0, SECOND, SECOND, 1.0);
    let after_one = project::encode(s.project(), false).unwrap();
    // Four keys: the level at the attack start, -12 dB over the take, the level at release end.
    assert!((level(&s, ItemId(5), SECOND / 2) + 12.0).abs() < 1e-9);
    assert!((level(&s, ItemId(5), 2 * SECOND) - 0.0).abs() < 1e-9);
    // 120 ms attack: halfway into the ramp the original is at -6 dB.
    let attack = 120 * (SECOND / 1000);
    assert!((level(&s, ItemId(5), -attack / 2) + 6.0).abs() < 1e-6);
    // 300 ms release: 150 ms after the take end it is at -6 dB again.
    assert!((level(&s, ItemId(5), SECOND + 150 * (SECOND / 1000)) + 6.0).abs() < 1e-6);

    // The same source cue appears again, overlapping the first take's release.
    let b = place(&mut s, 202, SECOND / 2, SECOND, SECOND, 1.0);
    assert_ne!(a, b);
    assert_eq!(narrations(&s).len(), 2);
    let after_two = project::encode(s.project(), false).unwrap();
    // The lowest gain wins where they overlap: still -12, never -24, through both takes.
    for at in [SECOND / 4, SECOND, SECOND * 6 / 5, SECOND * 7 / 5] {
        assert!((level(&s, ItemId(5), at) + 12.0).abs() < 1e-9, "{at}");
    }
    assert!((level(&s, ItemId(5), 2 * SECOND) - 0.0).abs() < 1e-9);

    // Disabling the second take's ducking leaves only the first take's envelope.
    s.apply(Command::DubSetDucking {
        item: b,
        ducking: NarrationDucking {
            enabled: false,
            ..NarrationDucking::DEFAULT
        },
    })
    .unwrap();
    assert!((level(&s, ItemId(5), SECOND + 150 * (SECOND / 1000)) + 6.0).abs() < 1e-6);
    // Ducking is adjustable: 6 dB deeper than the default, no attack.
    s.apply(Command::DubSetDucking {
        item: a,
        ducking: NarrationDucking {
            enabled: true,
            depth_tenths_db: 60,
            attack_ms: 0,
            release_ms: 300,
        },
    })
    .unwrap();
    assert!((level(&s, ItemId(5), SECOND / 2) + 6.0).abs() < 1e-9);

    // Every placement and every edit is one undo step, and redo restores it exactly.
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), after_two);
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), after_one);
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), start);
    s.redo().unwrap();
    s.redo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), after_two);
}

#[test]
fn provenance_and_the_consent_descriptor_round_trip_through_persistence() {
    let mut s = with_audio(&seed());
    let a = place(&mut s, 201, 0, SECOND, SECOND, 1.0);
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(loaded.project.data(), s.project().data());
    let stored = loaded
        .project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == a)
        .cloned()
        .unwrap();
    let ItemParams::Audio(AudioParams {
        source: AudioSource::Narration { narration },
        ..
    }) = stored.content
    else {
        panic!("narration item");
    };
    assert_eq!(narration, fx::narration(Tick(SECOND)));
    // A malformed hash is refused when the project is built, not repaired.
    let mut broken = narration;
    broken.text_sha256 = "not a hash".into();
    assert!(broken.validate().is_err());
}

#[test]
fn fit_changes_stay_inside_the_limits_and_overflow_only_into_silence() {
    let mut s = isolated();
    let original = project::encode(s.project(), false).unwrap();
    // 1.05x stretches through the ordinary retime path with pitch preserved.
    s.apply(Command::DubSetFit {
        item: ItemId(6),
        speed: time::Speed::from_f64(1.05).unwrap(),
    })
    .unwrap();
    let stretched = item(&s, ItemId(6));
    assert_eq!(stretched.retime.speed.thousandths(), 1050);
    assert!(stretched.retime.maintain_pitch);
    assert!(stretched.duration().unwrap() < Tick(SECOND));
    // Outside 0.90 to 1.10 is refused with the typed error and changes nothing.
    let held = project::encode(s.project(), false).unwrap();
    for speed in [1.2, 0.8] {
        let error = s
            .apply(Command::DubSetFit {
                item: ItemId(6),
                speed: time::Speed::from_f64(speed).unwrap(),
            })
            .unwrap_err();
        assert!(matches!(error, EditError::DurationFit), "{error:?}");
        assert_eq!(project::encode(s.project(), false).unwrap(), held);
    }
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), original);

    // 0.95x runs over the one-second cue; audible material right after it forbids that.
    let blocker = s
        .apply(Command::InsertItem {
            placement: Placement::Track { track: TrackId(5) },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(media::AssetId(97)),
                source_range: SourceRange::new(Tick::ZERO, Tick(SECOND)).unwrap(),
                start: Tick(2 * SECOND),
                content: ItemParams::Audio(AudioParams::default()),
            },
        })
        .unwrap()
        .created
        .iter()
        .find_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .unwrap();
    let slow = time::Speed::from_f64(0.95).unwrap();
    assert!(matches!(
        s.apply(Command::DubSetFit {
            item: ItemId(6),
            speed: slow
        }),
        Err(EditError::DurationFit)
    ));
    s.apply(Command::Delete {
        items: vec![blocker],
    })
    .unwrap();
    s.apply(Command::DubSetFit {
        item: ItemId(6),
        speed: slow,
    })
    .unwrap();
    assert!(item(&s, ItemId(6)).duration().unwrap() > Tick(SECOND));
}

#[test]
fn a_take_twice_its_cue_is_refused_and_places_nothing() {
    let mut s = with_audio(&seed());
    let before = project::encode(s.project(), false).unwrap();
    let history = s.history_len();
    // One second of audio for a half-second cue is 2x: beyond the stretch and overflow limits.
    let error = s
        .apply(Command::DubPlace {
            placement: Placement::Auto,
            start: Tick(5 * SECOND),
            asset: fx::asset(201, Tick(SECOND)),
            narration: fx::narration(Tick(SECOND / 2)),
            speed: time::Speed::UNITY,
        })
        .unwrap_err();
    assert!(matches!(error, EditError::DurationFit), "{error:?}");
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    assert_eq!(s.history_len(), history);
    assert!(narrations(&s).is_empty());
}

// ----- the credential opener: real adapters, no network -----

/// A transport that records every send; the opener and `prepare` must never reach it.
#[derive(Default)]
struct NoNetwork(std::sync::atomic::AtomicUsize);
impl transport::HttpTransport for NoNetwork {
    fn send(&self, _: transport::HttpRequest) -> Result<transport::HttpResponse, transport::Error> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(transport::Error::Network)
    }
}

fn real_request(provider: &str, model: &str, voice: &str, language: &str, text: &str) -> Request {
    Request {
        provider: provider.into(),
        model: model.into(),
        voice: voice.into(),
        language: LanguageTag::parse(language).unwrap(),
        text: text.into(),
        cue_duration: Tick(SECOND),
        provider_speed_thousandths: None,
    }
}

#[test]
fn the_credential_opener_builds_only_the_chosen_provider_with_its_own_key_and_no_network() {
    use engine::dubbing::{CredentialSpeech, SpeechOpener, speech_options};
    use engine::providers::Credentials;
    let wire = Arc::new(NoNetwork::default());

    // Only an OpenAI key: OpenAI opens, ElevenLabs is an authentication refusal, never a swap.
    let only_openai = CredentialSpeech::with_transport(
        Credentials::new(Some("sk-test".into()), None, None),
        wire.clone(),
    );
    let (synth, output) = only_openai.open("openai", "gpt-audio-1.5", None).unwrap();
    assert_eq!(synth.config().id().as_str(), "openai");
    assert_eq!(output.sample_rate, 24_000);
    assert!(matches!(
        only_openai.open("openai", "gpt-audio-1.5", Some(1100)),
        Err(ai::Error::Unsupported)
    ));
    assert!(matches!(
        only_openai.open("elevenlabs", "eleven_flash_v2_5", None),
        Err(ai::Error::Auth)
    ));
    assert!(matches!(
        only_openai.open("azure", "any", None),
        Err(ai::Error::Unsupported)
    ));

    // Both keys: ElevenLabs takes its speed setting inside 0.7 to 1.2 and refuses outside it.
    let both = CredentialSpeech::with_transport(
        Credentials::new(Some("sk-test".into()), Some("xi-test".into()), None),
        wire.clone(),
    );
    assert!(
        both.open("elevenlabs", "eleven_flash_v2_5", Some(1100))
            .is_ok()
    );
    assert!(
        both.open("elevenlabs", "eleven_flash_v2_5", Some(1300))
            .is_err()
    );

    // The narrator quotes and discloses through the real adapters before anything is sent.
    let narrator = Narrator::new(Arc::new(both));
    let flash = narrator
        .prepare(&real_request(
            "elevenlabs",
            "eleven_flash_v2_5",
            "21m00Tcm4TlvDq8ikWAM",
            "vi",
            "Xin chào",
        ))
        .unwrap();
    assert_eq!(flash.disclosure().provider, "elevenlabs");
    assert_eq!(
        flash.disclosure().cost,
        Cost::UpperBound {
            currency: "USD".into(),
            micros: 320
        }
    );
    // Multilingual v2 has no Vietnamese, an unknown voice is not guessed, and neither asks.
    assert!(matches!(
        narrator.prepare(&real_request(
            "elevenlabs",
            "eleven_multilingual_v2",
            "21m00Tcm4TlvDq8ikWAM",
            "vi",
            "Xin chào"
        )),
        Err(Error::Ai(ai::Error::UnsupportedLanguage))
    ));
    assert!(matches!(
        narrator.prepare(&real_request(
            "elevenlabs",
            "eleven_flash_v2_5",
            "cloned-voice",
            "en",
            "Hello"
        )),
        Err(Error::Ai(ai::Error::UnsupportedVoice))
    ));
    let openai = narrator
        .prepare(&real_request(
            "openai",
            "gpt-audio-1.5",
            "alloy",
            "en",
            "Welcome back.",
        ))
        .unwrap();
    assert_eq!(openai.disclosure().voice, "alloy");
    assert!(matches!(
        openai.disclosure().cost,
        Cost::UpperBound { micros, .. } if micros > 0
    ));
    assert_eq!(wire.0.load(std::sync::atomic::Ordering::SeqCst), 0);

    // The picker lists exactly these two providers, with built-in or premade voices only.
    let options = speech_options();
    assert_eq!(
        options.iter().map(|option| option.id).collect::<Vec<_>>(),
        ["openai", "elevenlabs"]
    );
    assert!(options.iter().all(|option| !option.models.is_empty()));
    assert!(options[0].voices.iter().any(|(id, _)| *id == "alloy"));
    assert!(options[1].speed && !options[0].speed);
}

// ----- dub.generate on a text item: the one path the panel, an agent and the CLI share -----

#[test]
fn dub_generate_on_a_text_item_runs_consent_in_the_job_and_returns_the_take_the_panel_shows() {
    use crate::support::control::{Allow, caller, finish, ok, request, runner, with_text};
    use engine::control::{ControlResult, ResponseValue};
    use engine::dubbing::decode_take;
    use engine::jobs::JobState;
    use engine::subtitles::CaptionAccess;

    let mut s = with_text(&seed());
    let before = project::encode(s.project(), false).unwrap();
    // The text item's own words and time are the source of the narration.
    let found = item(&s, ItemId(3));
    let ItemParams::Text(params) = &found.content else {
        panic!("text item");
    };
    let text = params.content.as_str().to_owned();
    let cue = found.duration().unwrap();
    let params = |language: &str| {
        serde_json::json!({
            "provider": "scripted", "model": "model-a", "voice": "alloy",
            "language": language, "text": text, "cue_duration": cue.0,
        })
    };
    let access_over = |voice: &Arc<Voice>, prompt: &Arc<Prompt>| {
        CaptionAccess::consent_only(prompt.clone()).with_dubbing(Arc::new(Narrator::new(Arc::new(
            Opener::new(vec![("scripted", "model-a", voice.clone())]),
        ))))
    };

    // Allowed: the job is accepted with no consent asked yet, then asks inside the job.
    let allowed_voice = fx_voice();
    let prompt = Arc::new(Prompt::new(true, false));
    let access = access_over(&allowed_voice, &prompt);
    let mut r = runner();
    let mut c = caller(&Allow);
    c.captions = Some(&access);
    let response = s
        .control_request(
            request(&s, "dub.generate", params("vi"), "gen-allow"),
            c,
            &mut r,
        )
        .unwrap();
    let ControlResult::JobAccepted { job, .. } = ok(response) else {
        panic!("dub.generate starts a job");
    };
    finish(&mut r, job);
    assert_eq!(r.snapshot(job).unwrap().state, JobState::Completed);
    let take = decode_take(r.output_bytes(job).unwrap()).unwrap();
    assert_eq!(prompt.count(), 1);
    assert_eq!(allowed_voice.calls(), 1);
    // One second of audio for the one-second text item, with the provenance of what was approved.
    assert_eq!(take.natural().unwrap(), cue);
    assert_eq!(take.narration.cue_duration, cue);
    assert_eq!(take.narration.characters, 18);
    assert_eq!(take.narration.text_sha256.as_str().len(), 64);
    assert_eq!(take.narration.voice.as_str(), "alloy");
    assert_eq!(project::encode(s.project(), false).unwrap(), before);

    // Denied: the job ends as a permission failure; nothing reached the provider.
    let denied_voice = fx_voice();
    let denied_prompt = Arc::new(Prompt::new(false, false));
    let denied = access_over(&denied_voice, &denied_prompt);
    let mut c = caller(&Allow);
    c.captions = Some(&denied);
    let response = s
        .control_request(
            request(&s, "dub.generate", params("vi"), "gen-deny"),
            c,
            &mut r,
        )
        .unwrap();
    let ControlResult::JobAccepted { job, .. } = ok(response) else {
        panic!("dub.generate starts a job");
    };
    finish(&mut r, job);
    let snapshot = r.snapshot(job).unwrap();
    assert_eq!(snapshot.state, JobState::Failed);
    assert_eq!(
        snapshot.error.unwrap().code(),
        engine::control::ErrorCode::Permission
    );
    assert_eq!((denied_prompt.count(), denied_voice.calls()), (1, 0));

    // An unsupported language is refused by the command itself: no job, no question.
    let refused_prompt = Arc::new(Prompt::new(true, false));
    let refused = access_over(&fx_voice(), &refused_prompt);
    let mut c = caller(&Allow);
    c.captions = Some(&refused);
    let response = s
        .control_request(
            request(&s, "dub.generate", params("zh"), "gen-zh"),
            c,
            &mut r,
        )
        .unwrap();
    assert!(
        matches!(response.value, ResponseValue::Error(_)),
        "{response:?}"
    );
    assert_eq!(refused_prompt.count(), 0);
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}
fn fx_voice() -> Arc<Voice> {
    voice(Script::Samples(24_000))
}

// ----- live: one short take per provider through the shipped credential opener -----

/// One `dub.generate` through `CredentialSpeech` with the production transport and the
/// provider's default model; the take must decode and last a plausible time for its text.
fn live_take(provider: &str, voice: &str) {
    use crate::support::control::{Allow, caller, finish, ok, request, runner, with_text};
    use engine::control::ControlResult;
    use engine::dubbing::{CredentialSpeech, decode_take};
    use engine::jobs::JobState;
    use engine::providers::Credentials;
    use engine::subtitles::CaptionAccess;

    if std::env::var("VIBECUT_LIVE_AI").as_deref() != Ok("1") {
        eprintln!("live test skipped: set VIBECUT_LIVE_AI=1 to run");
        return;
    }
    let credentials = Credentials::new(
        std::env::var("OPENAI_API_KEY").ok(),
        std::env::var("ELEVENLABS_API_KEY").ok(),
        None,
    );
    let model = ai::defaults::model(provider, ai::Operation::Synthesize);
    let text = "Welcome back. This is a short narration test.";
    let cue = Tick(3 * SECOND);
    let prompt = Arc::new(Prompt::new(true, true));
    let access = CaptionAccess::consent_only(prompt.clone()).with_dubbing(Arc::new(Narrator::new(
        Arc::new(CredentialSpeech::new(credentials)),
    )));
    let mut s = with_text(&seed());
    let mut r = runner();
    let mut c = caller(&Allow);
    c.captions = Some(&access);
    let params = serde_json::json!({
        "provider": provider, "model": model, "voice": voice,
        "language": "en", "text": text, "cue_duration": cue.0,
    });
    let response = s
        .control_request(request(&s, "dub.generate", params, "live"), c, &mut r)
        .unwrap();
    let ControlResult::JobAccepted { job, .. } = ok(response) else {
        panic!("dub.generate starts a job");
    };
    finish(&mut r, job);
    let snapshot = r.snapshot(job).unwrap();
    assert_eq!(snapshot.state, JobState::Completed, "{:?}", snapshot.error);
    let take = decode_take(r.output_bytes(job).unwrap()).unwrap();
    let natural = take.natural().unwrap();
    assert!(
        (Tick(SECOND)..=Tick(12 * SECOND)).contains(&natural),
        "{provider}: {natural:?} for {} characters",
        text.len()
    );
    assert!(take.pcm.iter().any(|sample| sample.unsigned_abs() > 1_000));
    assert_eq!(take.narration.provider.as_str(), provider);
    assert_eq!(prompt.count(), 1);
}

#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1 and OPENAI_API_KEY; one short paid speech request"]
fn live_openai_take_through_the_credential_opener() {
    live_take("openai", "alloy");
}

#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1 and ELEVENLABS_API_KEY; one short paid speech request"]
fn live_elevenlabs_take_through_the_credential_opener() {
    live_take("elevenlabs", "21m00Tcm4TlvDq8ikWAM");
}

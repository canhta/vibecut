#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use ai::speech::*;
use ai::{Error, ExecutionHooks, Location, Money, ProviderConfig, Stage, Support};
use captions::LanguageTag;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn stage(&self, _: Stage) {}
}
#[derive(Clone, Copy)]
enum Fixture {
    Success,
    Mismatch,
    Empty,
    Nan,
    Infinite,
    Truncated,
    WrongRate,
    WrongIdentity,
    TooLong,
    Overspend,
    Auth,
    Quota,
    RateLimit,
    ContentPolicy,
    Network,
    Timeout,
    Server,
    Cancelled,
    CancelAfter,
}
struct Fake {
    caps: SpeechCapabilities,
    estimate: SynthesisEstimate,
    calls: AtomicUsize,
    fixture: Fixture,
}
fn money(n: u64) -> Money {
    Money::new("USD", n).unwrap()
}
fn request(text: &str) -> SpeechRequest {
    SpeechRequest::new(
        ProviderConfig::new(
            "fake".into(),
            "model".into(),
            "settings".into(),
            Location::Remote {
                backend: "fixture".into(),
                base_url: "https://example.com/v1".into(),
                auth_ref: "secret-reference".into(),
            },
        )
        .unwrap(),
        text.into(),
        "stock".into(),
        SpeechLanguage {
            language: LanguageTag::parse("vi").unwrap(),
            script: "Latn".into(),
        },
        SpeechOutput {
            format: SpeechFormat::Pcm16,
            sample_rate: 24_000,
        },
        1_000_000,
    )
    .unwrap()
}
fn fake(r: &SpeechRequest, fixture: Fixture) -> Fake {
    Fake {
        caps: SpeechCapabilities {
            provider: r.provider().clone(),
            fingerprint: "exact-model-voice-language-format-settings".into(),
            languages: vec![r.language().clone()],
            voices: vec![SpeechVoice {
                id: r.voice().clone(),
                category: VoiceCategory::Stock,
            }],
            outputs: vec![r.output()],
            max_characters: Some(100),
            streaming: Support::Unsupported,
            max_concurrency: Some(1),
            requests_per_minute: None,
            cost_model: "bounded USD invocation".into(),
            policy: SpeechPolicy {
                policy_url: "https://example.com/privacy".into(),
                retention: "fixture retention".into(),
                training: "fixture training".into(),
                voice_disclosure: "synthetic voice".into(),
            },
        },
        estimate: SynthesisEstimate::UpperBound(money(10)),
        calls: AtomicUsize::new(0),
        fixture,
    }
}
fn consent(p: &Fake, r: &SpeechRequest) -> SynthesisConsent {
    SynthesisConsent::grant(r, &p.caps, p.estimate.clone(), money(10)).unwrap()
}
impl SpeechSynthesizer for Fake {
    fn config(&self) -> &ProviderConfig {
        &self.caps.provider
    }
    fn capabilities(&self, _: &SpeechRequest) -> SpeechCapabilities {
        self.caps.clone()
    }
    fn estimate(&self, _: &SpeechRequest) -> SynthesisEstimate {
        self.estimate.clone()
    }
    fn synthesize(
        &self,
        r: &SpeechRequest,
        c: &SynthesisConsent,
        h: &dyn ExecutionHooks,
    ) -> Result<SpeechAudio, Error> {
        prepare_synthesis(self, r, c)?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut audio = SpeechAudio {
            samples: vec![0.25; 24],
            sample_rate: 24_000,
            duration_micros: 1000,
            fingerprint: self.caps.fingerprint.clone(),
            usage: SpeechUsage {
                billable_characters: Some(r.text().as_str().chars().count() as u64),
                input_tokens: Some(1),
                output_tokens: Some(1),
                actual_cost: money(5),
            },
            transcript: Some(r.text().clone()),
        };
        match self.fixture {
            Fixture::Success => {}
            Fixture::Mismatch => audio.transcript = Some("different text".into()),
            Fixture::Empty => audio.samples.clear(),
            Fixture::Nan => audio.samples[0] = f32::NAN,
            Fixture::Infinite => audio.samples[0] = f32::INFINITY,
            Fixture::Truncated => {
                audio.samples.pop();
            }
            Fixture::WrongRate => audio.sample_rate = 0,
            Fixture::WrongIdentity => audio.fingerprint = "another model".into(),
            Fixture::TooLong => {
                audio.samples = vec![0.; 24_001];
                audio.duration_micros = 1_000_042;
            }
            Fixture::Overspend => audio.usage.actual_cost = money(11),
            Fixture::Auth => return Err(Error::Auth),
            Fixture::Quota => return Err(Error::Quota),
            Fixture::RateLimit => {
                return Err(Error::RateLimit {
                    retry_after_seconds: Some(3),
                });
            }
            Fixture::ContentPolicy => return Err(Error::ContentPolicy),
            Fixture::Network => return Err(Error::Network),
            Fixture::Timeout => return Err(Error::Timeout),
            Fixture::Server => return Err(Error::Server),
            Fixture::Cancelled => return Err(Error::Cancelled),
            Fixture::CancelAfter => {
                h.progress(Stage::Assemble, 100);
            }
        }
        Ok(audio)
    }
}
#[test]
fn production_seam_success_nfc_and_redaction() {
    let r = request("Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t");
    let p = fake(&r, Fixture::Success);
    let c = consent(&p, &r);
    let audio = synthesize(&p, &r, &c, &Hooks::default()).unwrap();
    assert_eq!(audio.transcript.as_ref(), Some(r.text()));
    assert_eq!(r, request("Tiếng Việt"));
    assert_eq!(r.characters(), 10);
    assert_eq!(r.text_sha256(), request("Tiếng Việt").text_sha256());
    assert_eq!(r.operation(), ai::Operation::Synthesize);
    for debug in [format!("{r:?}"), format!("{c:?}"), format!("{audio:?}")] {
        assert!(!debug.contains("Tiếng"));
        assert!(!debug.contains("secret-reference"));
        assert!(!debug.contains("0.25"));
    }
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn language_voice_payload_and_budget_fail_before_adapter() {
    let r = request("Tiếng Việt");
    for kind in 0..6 {
        let mut p = fake(&r, Fixture::Success);
        let c = consent(&p, &r);
        match kind {
            0 => p.caps.languages.clear(),
            1 => p.caps.voices.clear(),
            2 => p.caps.max_characters = Some(1),
            3 => p.estimate = SynthesisEstimate::Unknown,
            4 => p.estimate = SynthesisEstimate::UpperBound(money(11)),
            _ => p.caps.outputs.clear(),
        }
        let err = synthesize(&p, &r, &c, &Hooks::default()).unwrap_err();
        match kind {
            0 => assert!(matches!(err, Error::UnsupportedLanguage)),
            1 => assert!(matches!(err, Error::UnsupportedVoice)),
            2 => assert!(matches!(err, Error::Payload)),
            3 | 4 => assert!(matches!(err, Error::ConsentRequired | Error::Cost)),
            _ => assert!(matches!(err, Error::Unsupported)),
        }
        assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    }
    let p = fake(&r, Fixture::Success);
    for quote in [
        SynthesisEstimate::Unknown,
        SynthesisEstimate::Exact(money(11)),
        SynthesisEstimate::UpperBound(Money::new("EUR", 1).unwrap()),
    ] {
        assert!(matches!(
            SynthesisConsent::grant(&r, &p.caps, quote, money(10)),
            Err(Error::Cost)
        ));
    }
}
#[test]
fn every_scope_policy_and_quote_change_invalidates_consent() {
    let r = request("Tiếng Việt");
    let p = fake(&r, Fixture::Success);
    let c = consent(&p, &r);
    let rebuild = |text, voice, lang, output, duration, provider| {
        SpeechRequest::new(provider, text, voice, lang, output, duration).unwrap()
    };
    let mut provider = r.provider().clone();
    for kind in 0..16 {
        let mut p = fake(&r, Fixture::Success);
        let mut lang = r.language().clone();
        let mut out = r.output();
        let (mut text, mut voice, mut duration) =
            (r.text().clone(), r.voice().clone(), r.max_duration_micros());
        match kind {
            0 => text = "different".into(),
            1 => {
                voice = "other".into();
                p.caps.voices.push(SpeechVoice {
                    id: voice.clone(),
                    category: VoiceCategory::BuiltIn,
                });
            }
            2 => {
                lang.language = LanguageTag::parse("en").unwrap();
                p.caps.languages.push(lang.clone());
            }
            3 => {
                lang.script = "Hant".into();
                p.caps.languages.push(lang.clone());
            }
            4 => {
                out.format = SpeechFormat::WavPcm16;
                p.caps.outputs.push(out);
            }
            5 => {
                out.sample_rate = 48_000;
                p.caps.outputs.push(out);
            }
            6 => duration -= 1,
            7 => p.caps.policy.retention = "changed retention".into(),
            8 => p.caps.policy.training = "changed training".into(),
            9 => p.estimate = SynthesisEstimate::Exact(money(9)),
            10 => p.caps.fingerprint = "new revision".into(),
            11 => {
                provider = ProviderConfig::new(
                    "fake".into(),
                    "model".into(),
                    "changed".into(),
                    r.provider().location().clone(),
                )
                .unwrap()
            }
            12 => {
                provider = ProviderConfig::new(
                    "other-provider".into(),
                    "model".into(),
                    "settings".into(),
                    r.provider().location().clone(),
                )
                .unwrap()
            }
            13 => {
                provider = ProviderConfig::new(
                    "fake".into(),
                    "other-model".into(),
                    "settings".into(),
                    r.provider().location().clone(),
                )
                .unwrap()
            }
            14 => {
                provider = ProviderConfig::new(
                    "fake".into(),
                    "model".into(),
                    "settings".into(),
                    Location::Remote {
                        backend: "fixture".into(),
                        base_url: "https://other.example.com/v1".into(),
                        auth_ref: "secret-reference".into(),
                    },
                )
                .unwrap()
            }
            _ => {
                provider = ProviderConfig::new(
                    "fake".into(),
                    "model".into(),
                    "settings".into(),
                    Location::Remote {
                        backend: "fixture".into(),
                        base_url: "https://example.com/v1".into(),
                        auth_ref: "other-reference".into(),
                    },
                )
                .unwrap()
            }
        }
        let changed = rebuild(text, voice, lang, out, duration, provider.clone());
        assert!(
            matches!(
                synthesize(&p, &changed, &c, &Hooks::default()),
                Err(Error::ConsentRequired)
            ),
            "case {kind}"
        );
        assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn hostile_output_and_truncation_are_rejected() {
    let r = request("text");
    for fixture in [
        Fixture::Mismatch,
        Fixture::Empty,
        Fixture::Nan,
        Fixture::Infinite,
        Fixture::Truncated,
        Fixture::WrongRate,
        Fixture::WrongIdentity,
        Fixture::TooLong,
    ] {
        let p = fake(&r, fixture);
        assert!(matches!(
            synthesize(&p, &r, &consent(&p, &r), &Hooks::default()),
            Err(Error::OutputRejected)
        ));
    }
    let p = fake(&r, Fixture::Overspend);
    assert!(matches!(
        synthesize(&p, &r, &consent(&p, &r), &Hooks::default()),
        Err(Error::Cost)
    ));
}
#[test]
fn typed_errors_are_safe_single_attempt_failures() {
    assert!(Error::Timeout.is_transient());
    assert!(!Error::ContentPolicy.is_transient());
    let r = request("private text");
    for (fixture, expected) in [
        (Fixture::Auth, Error::Auth),
        (Fixture::Quota, Error::Quota),
        (
            Fixture::RateLimit,
            Error::RateLimit {
                retry_after_seconds: Some(3),
            },
        ),
        (Fixture::ContentPolicy, Error::ContentPolicy),
        (Fixture::Network, Error::Network),
        (Fixture::Timeout, Error::Timeout),
        (Fixture::Server, Error::Server),
        (Fixture::Cancelled, Error::Cancelled),
    ] {
        let p = fake(&r, fixture);
        let error = synthesize(&p, &r, &consent(&p, &r), &Hooks::default()).unwrap_err();
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected)
        );
        assert!(!error.to_string().contains("private"));
        assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    }
}
struct CancelAfter(AtomicBool);
impl ExecutionHooks for CancelAfter {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn stage(&self, _: Stage) {}
    fn progress(&self, _: Stage, _: u8) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[test]
fn cancellation_before_and_after_io_discards_audio() {
    let r = request("text");
    let p = fake(&r, Fixture::Success);
    assert!(matches!(
        synthesize(&p, &r, &consent(&p, &r), &Hooks(AtomicBool::new(true))),
        Err(Error::Cancelled)
    ));
    assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    let p = fake(&r, Fixture::CancelAfter);
    assert!(matches!(
        synthesize(
            &p,
            &r,
            &consent(&p, &r),
            &CancelAfter(AtomicBool::new(false))
        ),
        Err(Error::Cancelled)
    ));
}
#[test]
fn constructor_boundaries_and_exact_graphemes() {
    let r = request("a\u{0301}👩‍💻中");
    assert_eq!(r.characters(), 3);
    assert_ne!(r.text_sha256(), request("other").text_sha256());
    for (text, rate, duration) in [
        ("", 24_000, 1),
        (" ", 24_000, 1),
        ("a", 0, 1),
        ("a", 24_000, 0),
    ] {
        assert!(
            SpeechRequest::new(
                r.provider().clone(),
                text.into(),
                r.voice().clone(),
                r.language().clone(),
                SpeechOutput {
                    format: SpeechFormat::Pcm16,
                    sample_rate: rate
                },
                duration
            )
            .is_err()
        );
    }
    let large = "a".repeat(MAX_SPEECH_TEXT_BYTES + 1);
    assert!(matches!(
        SpeechRequest::new(
            r.provider().clone(),
            large.into(),
            r.voice().clone(),
            r.language().clone(),
            r.output(),
            1
        ),
        Err(Error::Payload)
    ));
}
#[test]
fn free_exact_optional_transcript_and_duration_rounding() {
    let r = request("text");
    let mut p = fake(&r, Fixture::Success);
    p.estimate = SynthesisEstimate::Exact(money(5));
    let c = consent(&p, &r);
    let mut audio = synthesize(&p, &r, &c, &Hooks::default()).unwrap();
    audio.samples = vec![0.];
    audio.duration_micros = 42;
    audio.transcript = None;
    validate_speech_audio(&audio, &r, &p.caps, &p.estimate).unwrap();
    audio.usage.actual_cost = money(0);
    validate_speech_audio(&audio, &r, &p.caps, &SynthesisEstimate::Free).unwrap();
    audio.duration_micros = 41;
    assert!(matches!(
        validate_speech_audio(&audio, &r, &p.caps, &SynthesisEstimate::Free),
        Err(Error::OutputRejected)
    ));
}

#[test]
fn pcm_memory_ceiling_and_exact_duration_boundary() {
    let r = request("text");
    let p = fake(&r, Fixture::Success);
    let mut audio = synthesize(&p, &r, &consent(&p, &r), &Hooks::default()).unwrap();
    audio.samples = vec![0.; 24_000];
    audio.duration_micros = 1_000_000;
    validate_speech_audio(&audio, &r, &p.caps, &p.estimate).unwrap();
    audio.samples = vec![0.; (ai::MAX_PCM_BYTES / 4 + 1) as usize];
    assert!(matches!(
        validate_speech_audio(&audio, &r, &p.caps, &p.estimate),
        Err(Error::OutputRejected)
    ));
}

#[test]
fn exact_text_limit_and_capability_policy_validation() {
    let r = request("text");
    let boundary = SpeechRequest::new(
        r.provider().clone(),
        "a".repeat(MAX_SPEECH_TEXT_BYTES).into(),
        r.voice().clone(),
        r.language().clone(),
        r.output(),
        1,
    )
    .unwrap();
    assert_eq!(boundary.characters(), MAX_SPEECH_TEXT_BYTES as u64);
    let mut p = fake(&r, Fixture::Success);
    p.caps.max_characters = Some(4);
    consent(&p, &r);
    p.caps.max_characters = Some(3);
    assert!(matches!(
        SynthesisConsent::grant(&r, &p.caps, p.estimate.clone(), money(10)),
        Err(Error::Payload)
    ));
    for url in [
        "http://example.com/policy",
        "https://key@example.com/policy",
        "not a url",
    ] {
        let mut p = fake(&r, Fixture::Success);
        p.caps.policy.policy_url = url.into();
        assert!(SynthesisConsent::grant(&r, &p.caps, p.estimate.clone(), money(10)).is_err());
    }
    let mut p = fake(&r, Fixture::Success);
    p.estimate = SynthesisEstimate::Free;
    let c = consent(&p, &r);
    assert!(matches!(
        synthesize(&p, &r, &c, &Hooks::default()),
        Err(Error::Cost)
    ));
}

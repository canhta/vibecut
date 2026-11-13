//! Upload consent shows each provider's own disclosure, field by field.
#![allow(clippy::unwrap_used, reason = "offline consent fixtures")]
use ai::foundation::transport::ApiKey;
use ai::{
    Location, ProviderConfig, SampleRange, Scope, Transcriber, TranslationSegment,
    TranslationWindow, Translator,
};
use captions::LanguageTag;
use engine::providers::Error as ProviderError;
use engine::providers::{CatalogOperation, ConsentPrompt, Cost, StoredConsent, UploadRequest};
use std::sync::Arc;
use transport::{HttpRequest, HttpResponse, HttpTransport};

struct NoTransport;
impl HttpTransport for NoTransport {
    fn send(&self, _: HttpRequest) -> Result<HttpResponse, transport::Error> {
        Err(transport::Error::Network)
    }
}

/// What the consent dialog shows for one provider.
struct Row {
    name: &'static str,
    operation: CatalogOperation,
    provider: &'static str,
    model: String,
    languages: &'static str,
    audio_seconds: f64,
    upload_bytes: u64,
    cost: Cost,
    terms: Vec<(&'static str, &'static str)>,
}

fn tag(value: &str) -> LanguageTag {
    LanguageTag::parse(value).unwrap()
}
fn usd(micros: u64) -> Cost {
    Cost::UpperBound {
        currency: "USD".into(),
        micros,
    }
}
/// Two seconds of 16 kHz audio.
fn audio_scope(config: ProviderConfig, mode: &str) -> Scope {
    Scope::transcription(
        config,
        mode.into(),
        tag("vi"),
        SampleRange::new(0, 32_000).unwrap(),
        16_000,
        "fingerprint".into(),
    )
    .unwrap()
}
fn window() -> TranslationWindow {
    TranslationWindow {
        source_language: tag("vi"),
        target_language: tag("en"),
        segments: vec![TranslationSegment {
            id: "cue-1".into(),
            text: "xin chào".into(),
        }],
        before: Vec::new(),
        after: Vec::new(),
        glossary: Vec::new(),
        style: "subtitle".into(),
    }
}
fn translation_scope(config: &ProviderConfig, mode: &str, window: &TranslationWindow) -> Scope {
    Scope::translation(config.clone(), mode.into(), window, "fp".into()).unwrap()
}
fn transcription_request(adapter: &dyn Transcriber, scope: &Scope) -> UploadRequest {
    UploadRequest::for_scope(
        CatalogOperation::Transcribe,
        adapter.config(),
        adapter.disclosure(scope).unwrap(),
        &adapter.estimate(scope),
        scope,
    )
    .unwrap()
}
fn translation_request(adapter: &dyn Translator, scope: &Scope) -> UploadRequest {
    UploadRequest::for_scope(
        CatalogOperation::Translate,
        adapter.config(),
        adapter.disclosure(scope).unwrap(),
        &adapter.estimate(scope),
        scope,
    )
    .unwrap()
}

#[test]
fn every_provider_shows_its_own_disclosure_fields() {
    let key = || ApiKey::new("test").unwrap();
    let transport = || -> Arc<dyn HttpTransport> { Arc::new(NoTransport) };

    let openai = ai::providers::openai::OpenAiTranscriber::new(
        ai::providers::openai::OpenAiTranscriber::provider_config("gpt-transcribe").unwrap(),
        key(),
        transport(),
    )
    .unwrap();
    let openai_scope = audio_scope(openai.config().clone(), ai::providers::openai::MODE);
    let elevenlabs = ai::providers::elevenlabs::ElevenLabsTranscriber::new(
        ai::providers::elevenlabs::ElevenLabsTranscriber::provider_config("scribe_v2").unwrap(),
        key(),
        transport(),
    )
    .unwrap();
    let elevenlabs_scope =
        audio_scope(elevenlabs.config().clone(), ai::providers::elevenlabs::MODE);
    let openai_translator = ai::providers::openai::OpenAiTranslator::new(
        ai::providers::openai::OpenAiTranslator::provider_config("gpt-6.1-sol").unwrap(),
        key(),
        transport(),
    )
    .unwrap();
    let anthropic_translator = ai::providers::anthropic::AnthropicTranslator::new(
        ai::providers::anthropic::AnthropicTranslator::provider_config("claude-sonnet-5-5")
            .unwrap(),
        key(),
        transport(),
    )
    .unwrap();
    let to_en = window();
    let payload_bytes = to_en.consent_text().unwrap().as_str().len() as u64;
    let openai_translation = translation_scope(
        openai_translator.config(),
        ai::providers::openai::TRANSLATION_MODE,
        &to_en,
    );
    let anthropic_translation = translation_scope(
        anthropic_translator.config(),
        ai::providers::anthropic::TRANSLATION_MODE,
        &to_en,
    );

    let rows = [
        (
            Row {
                name: "openai transcription",
                operation: CatalogOperation::Transcribe,
                provider: "OpenAI",
                model: "gpt-transcribe".into(),
                languages: "vi",
                audio_seconds: 2.0,
                upload_bytes: 64_044,
                cost: usd(150),
                terms: vec![
                    (
                        "policy",
                        "https://developers.openai.com/api/docs/guides/your-data",
                    ),
                    (
                        "docs",
                        "https://developers.openai.com/api/docs/guides/speech-to-text",
                    ),
                ],
            },
            transcription_request(&openai, &openai_scope),
        ),
        (
            Row {
                name: "elevenlabs transcription",
                operation: CatalogOperation::Transcribe,
                provider: "ElevenLabs",
                model: "scribe_v2".into(),
                languages: "vi",
                audio_seconds: 2.0,
                upload_bytes: 64_044,
                cost: usd(123),
                terms: vec![
                    ("policy", "https://elevenlabs.io/privacy-policy"),
                    (
                        "pricing",
                        "https://elevenlabs.io/pricing/api?price.section=speech_to_text",
                    ),
                    (
                        "docs",
                        "https://elevenlabs.io/docs/capabilities/speech-to-text",
                    ),
                ],
            },
            transcription_request(&elevenlabs, &elevenlabs_scope),
        ),
        (
            Row {
                name: "openai translation",
                operation: CatalogOperation::Translate,
                provider: "OpenAI",
                model: "gpt-6.1-sol".into(),
                languages: "vi -> en",
                audio_seconds: 0.0,
                upload_bytes: payload_bytes,
                cost: Cost::from(&openai_translator.estimate(&openai_translation)),
                terms: vec![(
                    "policy",
                    "https://developers.openai.com/api/docs/guides/your-data",
                )],
            },
            translation_request(&openai_translator, &openai_translation),
        ),
        (
            Row {
                name: "anthropic translation",
                operation: CatalogOperation::Translate,
                provider: "Anthropic",
                model: "claude-sonnet-5-5".into(),
                languages: "vi -> en",
                audio_seconds: 0.0,
                upload_bytes: payload_bytes,
                cost: Cost::from(&anthropic_translator.estimate(&anthropic_translation)),
                terms: vec![
                    (
                        "policy",
                        "https://platform.claude.com/docs/en/manage-claude/api-and-data-retention",
                    ),
                    (
                        "pricing",
                        "https://platform.claude.com/docs/en/about-claude/pricing",
                    ),
                    (
                        "structured_output",
                        "https://platform.claude.com/docs/en/build-with-claude/structured-outputs",
                    ),
                ],
            },
            translation_request(&anthropic_translator, &anthropic_translation),
        ),
        // A composed remote provider without a disclosure is described from its
        // configuration and scope alone.
        (
            Row {
                name: "composed remote transcription",
                operation: CatalogOperation::Transcribe,
                provider: "openai",
                model: "gpt-transcribe".into(),
                languages: "vi",
                audio_seconds: 2.0,
                upload_bytes: 64_044,
                cost: Cost::Free,
                terms: vec![("policy", "test provider policy")],
            },
            UploadRequest::for_scope(
                CatalogOperation::Transcribe,
                openai.config(),
                None,
                &ai::CostEstimate::Free,
                &openai_scope,
            )
            .unwrap(),
        ),
        (
            Row {
                name: "composed remote translation",
                operation: CatalogOperation::Translate,
                provider: "openai",
                model: "gpt-6.1-sol".into(),
                languages: "vi -> en",
                audio_seconds: 0.0,
                upload_bytes: openai_translation.max_bytes,
                cost: Cost::Free,
                terms: vec![("policy", "test provider policy")],
            },
            UploadRequest::for_scope(
                CatalogOperation::Translate,
                openai_translator.config(),
                None,
                &ai::CostEstimate::Free,
                &openai_translation,
            )
            .unwrap(),
        ),
    ];
    for (row, actual) in rows {
        assert_eq!(actual.operation, row.operation, "{}", row.name);
        assert_eq!(actual.provider, row.provider, "{}", row.name);
        assert_eq!(actual.model, row.model, "{}", row.name);
        assert_eq!(actual.languages, row.languages, "{}", row.name);
        assert_eq!(actual.audio_seconds, row.audio_seconds, "{}", row.name);
        assert_eq!(actual.upload_bytes, row.upload_bytes, "{}", row.name);
        assert_eq!(actual.cost, row.cost, "{}", row.name);
        let terms: Vec<_> = actual
            .terms
            .iter()
            .map(|(label, url)| (*label, url.as_str()))
            .collect();
        assert_eq!(terms, row.terms, "{}", row.name);
    }
}

#[test]
fn a_local_provider_has_no_upload_to_consent_to() {
    let local = ProviderConfig::new(
        "local-whisper".into(),
        "whisper-large-v3-turbo".into(),
        "settings".into(),
        Location::Local {
            backend: "whisper.cpp".into(),
            model_ref: "sha".into(),
        },
    )
    .unwrap();
    let scope = audio_scope(local.clone(), "transcribe");
    let refused = UploadRequest::for_scope(
        CatalogOperation::Transcribe,
        &local,
        None,
        &ai::CostEstimate::Free,
        &scope,
    );
    assert!(matches!(
        refused,
        Err(ProviderError::Edit(engine::EditError::Unsupported(
            "local provider has no upload"
        )))
    ));
}

#[test]
fn stored_consent_requires_exact_configuration_and_cost_ceiling() {
    let config = ai::text::chapters::OpenAiChapterGenerator::provider_config("gpt-6-luna").unwrap();
    let consent = StoredConsent::grant([(config.clone(), ai::Money::new("USD", 10).unwrap())]);
    for (field, value) in [
        ("settings_fingerprint", "different-settings"),
        ("base_url", "https://another.example/v1"),
        ("auth_ref", "another-credential"),
        ("backend", "another-backend"),
    ] {
        let mut data = serde_json::to_value(&config).unwrap();
        if field == "settings_fingerprint" {
            data[field] = value.into();
        } else {
            data["location"][field] = value.into();
        }
        let other: ProviderConfig = serde_json::from_value(data).unwrap();
        let scope = audio_scope(other.clone(), "fixture");
        let request = UploadRequest::for_scope(
            CatalogOperation::GenerateChapters,
            &other,
            None,
            &ai::CostEstimate::Free,
            &scope,
        )
        .unwrap();
        assert!(
            matches!(
                consent.approve(&request),
                Err(ProviderError::ConsentRequired)
            ),
            "changed {field} reused an approval"
        );
    }
    let scope = audio_scope(config.clone(), "fixture");
    for (estimate, allowed) in [
        (ai::CostEstimate::Free, true),
        (
            ai::CostEstimate::UpperBound(ai::Money::new("USD", 10).unwrap()),
            true,
        ),
        (
            ai::CostEstimate::UpperBound(ai::Money::new("USD", 11).unwrap()),
            false,
        ),
        (
            ai::CostEstimate::UpperBound(ai::Money::new("EUR", 1).unwrap()),
            false,
        ),
        (ai::CostEstimate::Unknown, false),
    ] {
        let request = UploadRequest::for_scope(
            CatalogOperation::GenerateChapters,
            &config,
            None,
            &estimate,
            &scope,
        )
        .unwrap();
        let result = consent.approve(&request);
        if allowed {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(ProviderError::Ai(ai::Error::Cost))));
        }
    }
}
#[test]
fn unknown_model_upload_requires_separate_acknowledgement_and_rejects_forged_disclosure() {
    struct Explicit;
    impl ConsentPrompt for Explicit {
        fn approve(&self, _: &UploadRequest) -> engine::providers::Result<()> {
            Err(ai::Error::ConsentRequired.into())
        }
        fn approve_unknown_price(&self, _: &UploadRequest) -> engine::providers::Result<()> {
            Ok(())
        }
    }
    let config =
        ai::providers::openai::OpenAiTranscriber::provider_config("fixture-custom-asr").unwrap();
    let adapter = ai::providers::openai::OpenAiTranscriber::new(
        config.clone(),
        ApiKey::new("synthetic").unwrap(),
        Arc::new(NoTransport),
    )
    .unwrap();
    let scope = audio_scope(config.clone(), ai::providers::openai::MODE);
    let estimate = adapter.estimate(&scope);
    assert_eq!(estimate, ai::CostEstimate::Unknown);
    let request = UploadRequest::for_scope(
        CatalogOperation::Transcribe,
        &config,
        adapter.disclosure(&scope).unwrap(),
        &estimate,
        &scope,
    )
    .unwrap();
    assert!(matches!(
        engine::providers::approve_upload(
            &engine::providers::AllowAll,
            &request,
            &scope,
            &estimate
        ),
        Err(ProviderError::Ai(ai::Error::Cost))
    ));
    let consent =
        engine::providers::approve_upload(&Explicit, &request, &scope, &estimate).unwrap();
    ai::authorize(&scope, &estimate, Some(&consent)).unwrap();
    let mut changed = request.clone();
    changed.model = "fixture-other".into();
    assert!(matches!(
        engine::providers::approve_upload(&Explicit, &changed, &scope, &estimate),
        Err(ProviderError::Ai(ai::Error::ConsentRequired))
    ));
    changed = request.clone();
    changed.operation = CatalogOperation::Translate;
    assert!(engine::providers::approve_upload(&Explicit, &changed, &scope, &estimate).is_err());
    changed = request;
    changed.cost = Cost::Free;
    assert!(engine::providers::approve_upload(&Explicit, &changed, &scope, &estimate).is_err());
    let key = |config: &ProviderConfig| {
        engine::cache::CacheKey::new(engine::cache::KeyInputs {
            kind: engine::cache::ArtifactKind::AsrChunk,
            source: [1; 32],
            provider: config.id().as_str(),
            model: config.model().as_str(),
            version: config.settings_fingerprint().as_str(),
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })
        .unwrap()
    };
    let other =
        ai::providers::openai::OpenAiTranscriber::provider_config("fixture-other-asr").unwrap();
    assert_ne!(key(&config), key(&other));
}

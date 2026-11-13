//! Owns shared remote provider admission.
pub mod defaults;
mod models;
use crate::transport::wav_bytes;
use crate::{Capabilities, CostEstimate, Error, Location, Operation, ProviderConfig, Scope};
use ::transport::{HttpResponse, retry_after};
use captions::NfcText;
pub use models::{MAX_REMOTE_MODELS, list_remote_models};
use sha2::{Digest, Sha256};

/// What composition shows before any remote byte leaves the machine. One value
/// for every remote operation; local adapters return `None` instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadDisclosure {
    pub operation: Operation,
    pub provider: NfcText,
    pub model: NfcText,
    /// Exact request languages: one BCP 47 tag, or "source -> target".
    pub languages: NfcText,
    /// Source-audio duration in microseconds; zero for a text-only payload.
    pub audio_micros: u64,
    /// Exact wire payload bytes.
    pub upload_bytes: u64,
    pub estimate: CostEstimate,
    /// Provider-owned policy and documentation links in display order.
    pub links: Vec<(&'static str, NfcText)>,
}
/// Fields every remote adapter supplies to [`disclosure`].
pub(crate) struct Disclosure<'a> {
    pub operation: Operation,
    pub provider: &'a str,
    pub model: &'a NfcText,
    /// Exact request languages: one BCP 47 tag, or "source -> target".
    pub languages: NfcText,
    /// Source-audio duration in microseconds; zero for a text-only payload.
    pub audio_micros: u64,
    /// Exact wire payload bytes.
    pub upload_bytes: u64,
    pub estimate: CostEstimate,
    /// Provider-owned policy and documentation links in display order.
    pub links: &'a [(&'static str, &'a str)],
}

/// Build the provider-neutral disclosure an adapter returns through its trait.
pub(crate) fn disclosure(request: Disclosure<'_>) -> UploadDisclosure {
    UploadDisclosure {
        operation: request.operation,
        provider: request.provider.into(),
        model: request.model.clone(),
        languages: request.languages,
        audio_micros: request.audio_micros,
        upload_bytes: request.upload_bytes,
        estimate: request.estimate,
        links: request
            .links
            .iter()
            .map(|(name, url)| (*name, NfcText::from(*url)))
            .collect(),
    }
}
impl UploadDisclosure {
    /// The named provider link, e.g. `"policy"`.
    pub fn link(&self, name: &str) -> Option<&NfcText> {
        self.links
            .iter()
            .find(|(label, _)| *label == name)
            .map(|(_, url)| url)
    }
}

/// Admit exactly one remote provider configuration. `settings` is the adapter's
/// required settings fingerprint; `local` names the refusal for a local
/// configuration (`None` refuses it as unsupported).
pub(crate) fn admit(
    config: &ProviderConfig,
    settings: Option<&str>,
    backend: &str,
    base_url: &str,
    local: Option<&'static str>,
) -> Result<(), Error> {
    crate::defaults::validate_model(config.model().as_str())?;
    if let Some(settings) = settings
        && config.settings_fingerprint()
            != &settings_fingerprint(settings, config.model().as_str())?
    {
        return Err(Error::Unsupported);
    }
    match config.location() {
        Location::Remote {
            backend: actual,
            base_url: url,
            ..
        } => {
            if actual.as_str() != backend || url.as_str().trim_end_matches('/') != base_url {
                return Err(Error::Unsupported);
            }
            Ok(())
        }
        Location::Local { .. } => Err(local.map_or(Error::Unsupported, Error::Invalid)),
    }
}

/// The adapter's one admitted configuration for a model id.
pub(crate) fn provider_config(
    model: &str,
    settings: &str,
    backend: &str,
    base_url: &str,
    id: &str,
    auth_ref: &str,
) -> Result<ProviderConfig, Error> {
    crate::defaults::validate_model(model)?;
    let model = NfcText::from(model);
    let fingerprint = settings_fingerprint(settings, model.as_str())?;
    ProviderConfig::new(
        id.into(),
        model,
        fingerprint,
        Location::Remote {
            backend: backend.into(),
            base_url: base_url.into(),
            auth_ref: auth_ref.into(),
        },
    )
}

/// Keep the complete model/settings identity within downstream cache-version bounds.
/// JSON tuple framing distinguishes component boundaries, including embedded delimiters.
fn settings_fingerprint(settings: &str, model: &str) -> Result<NfcText, Error> {
    let canonical = serde_json::to_vec(&("remote-settings-v1", settings, model))
        .map_err(|_| Error::Invalid("remote settings identity"))?;
    let digest: String = Sha256::digest(canonical)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("sha256:{digest}").into())
}

/// The disclosure cost ceiling for a speech quote.
pub(crate) fn cost_ceiling(estimate: crate::speech::SynthesisEstimate) -> CostEstimate {
    match estimate {
        crate::speech::SynthesisEstimate::Free => CostEstimate::Free,
        crate::speech::SynthesisEstimate::Exact(money)
        | crate::speech::SynthesisEstimate::UpperBound(money) => CostEstimate::UpperBound(money),
        crate::speech::SynthesisEstimate::Unknown => CostEstimate::Unknown,
    }
}

/// Refuse a scope that is not bound to this adapter's configuration and capabilities.
pub(crate) fn validate_scope(
    config: &ProviderConfig,
    scope: &Scope,
    capabilities: &Capabilities,
) -> Result<(), Error> {
    if scope.provider != *config {
        return Err(Error::Unsupported);
    }
    capabilities.check(scope)
}

/// Transcription scopes carry no text payload.
pub(crate) fn validate_audio_scope(
    config: &ProviderConfig,
    scope: &Scope,
    capabilities: &Capabilities,
) -> Result<(), Error> {
    if scope.provider != *config {
        return Err(Error::Unsupported);
    }
    if !scope.texts.is_empty() {
        return Err(Error::Invalid("transcription text payload"));
    }
    capabilities.check(scope)
}

/// Text operations bind exactly one payload; a missing or extra text is unconsented.
pub(crate) fn single_text(scope: &Scope) -> Result<&NfcText, Error> {
    match scope.texts.as_slice() {
        [text] => Ok(text),
        _ => Err(Error::ConsentRequired),
    }
}

/// Classify only model/capability refusals. Provider text is inspected transiently,
/// never retained in the error or presented as untrusted instructions.
pub(crate) fn check_model_response(
    response: &HttpResponse,
    config: &ProviderConfig,
) -> Result<(), Error> {
    if !matches!(response.status, 400 | 404 | 422) {
        return Ok(());
    }
    let fields = error_fields(&response.body);
    // The fixed Anthropic Messages endpoint has no resource selector other
    // than the chosen model; its typed 404 remains actionable without prose.
    let model = (config.id().as_str() == "anthropic"
        && response.status == 404
        && fields.iter().any(|field| field == "not_found_error"))
        || matches(
            &fields,
            &[
                "model_not_found",
                "model_not_available",
                "invalid_model",
                "unknown_model",
                "model_not_supported",
                "model_not_exist",
            ],
        )
        || (matches(&fields, &["model"])
            && matches(
                &fields,
                &[
                    "does not exist",
                    "not_found_error",
                    "not found",
                    "not supported",
                    "does not support",
                    "unavailable",
                    "not available",
                    "no access",
                ],
            ));
    let capability = matches(
        &fields,
        &[
            "unsupported_value",
            "unsupported_parameter",
            "not supported",
            "does not support",
        ],
    ) && matches(
        &fields,
        &[
            "response_format",
            "json_schema",
            "structured",
            "audio",
            "modalities",
        ],
    );
    if model || capability {
        return Err(Error::ModelUnavailable {
            provider: config.id().clone(),
            model: config.model().clone(),
            message: if capability {
                "required capability is not supported"
            } else {
                "model is missing or inaccessible"
            },
        });
    }
    Ok(())
}

/// The disclosure language for a source/target operation.
pub(crate) fn language_pair(scope: &Scope) -> Result<NfcText, Error> {
    let target = scope
        .target_language
        .as_ref()
        .ok_or(Error::UnsupportedLanguage)?;
    Ok(NfcText::from(format!(
        "{} -> {}",
        scope.source_language.as_str(),
        target.as_str()
    )))
}

/// Source-audio duration in microseconds, truncated like the provider bills it.
pub(crate) fn duration_micros(scope: &Scope) -> Result<u64, Error> {
    let micros = u128::from(scope.source_samples.len())
        .checked_mul(1_000_000)
        .ok_or(Error::Payload)?
        / u128::from(scope.sample_rate);
    u64::try_from(micros).map_err(|_| Error::Payload)
}

/// Whole audio seconds rounded up; `None` when the sample rate is zero or the
/// duration cannot be represented.
pub(crate) fn rounded_up_seconds(scope: &Scope) -> Option<u128> {
    let samples = u128::from(scope.source_samples.len());
    let rate = u128::from(scope.sample_rate);
    if rate == 0 {
        return None;
    }
    Some(samples.checked_add(rate - 1)? / rate)
}

/// Encoded PCM16 WAV bytes, refused above the adapter's advertised ceiling.
pub(crate) fn encoded_bytes(scope: &Scope, ceiling: u64) -> Result<u64, Error> {
    let bytes = wav_bytes(scope.source_samples.len())?;
    if bytes > ceiling {
        return Err(Error::Payload);
    }
    Ok(bytes)
}

/// Lowercased provider error fields for classification only; bounded and never
/// logged. Reads the root envelope and its `error`/`detail` object.
pub(crate) fn error_fields(body: &[u8]) -> Vec<String> {
    const KEYS: [&str; 5] = ["code", "type", "message", "param", "status"];
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    for container in [Some(&value), value.get("error"), value.get("detail")] {
        let Some(container) = container else {
            continue;
        };
        for key in KEYS {
            if let Some(text) = container.get(key).and_then(serde_json::Value::as_str) {
                fields.push(
                    text.chars()
                        .take(128)
                        .collect::<String>()
                        .to_ascii_lowercase(),
                );
            }
        }
    }
    fields
}

/// True when any provider error field contains one of the lowercase needles.
pub(crate) fn matches(fields: &[String], needles: &[&str]) -> bool {
    fields
        .iter()
        .any(|field| needles.iter().any(|needle| field.contains(needle)))
}

/// Provider error envelope names exhausted credit, quota or billing.
pub(crate) fn quota(fields: &[String]) -> bool {
    matches(
        fields,
        &[
            "quota",
            "billing",
            "credit balance",
            "insufficient_credit",
            "payment_required",
        ],
    )
}

/// Shared status arms; `classify` owns the provider's body rules and any status
/// that maps differently from the common table.
pub(crate) fn status_error(
    response: &HttpResponse,
    classify: impl FnOnce(u16, &[String]) -> Option<Error>,
) -> Error {
    let fields = error_fields(&response.body);
    if let Some(error) = classify(response.status, &fields) {
        return error;
    }
    match response.status {
        401 | 403 => Error::Auth,
        413 => Error::Payload,
        429 => Error::RateLimit {
            retry_after_seconds: retry_after(&response.headers),
        },
        500..=599 => Error::Server,
        300..=399 => Error::Invalid("provider redirect"),
        400 | 404 | 409 | 422 => Error::Invalid("provider rejected request"),
        _ => Error::Server,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use ::transport::HttpResponse;
    use captions::LanguageTag;

    fn config(model: &str, settings: &str, backend: &str, base_url: &str) -> ProviderConfig {
        ProviderConfig::new(
            "provider".into(),
            model.into(),
            settings_fingerprint(settings, model).unwrap(),
            Location::Remote {
                backend: backend.into(),
                base_url: base_url.into(),
                auth_ref: "PROVIDER_API_KEY".into(),
            },
        )
        .unwrap()
    }

    fn local(model: &str, settings: &str) -> ProviderConfig {
        ProviderConfig::new(
            "provider".into(),
            model.into(),
            settings_fingerprint(settings, model).unwrap(),
            Location::Local {
                backend: "local".into(),
                model_ref: "sha256:model".into(),
            },
        )
        .unwrap()
    }

    const SETTINGS: &str = "settings-v1";
    const BACKEND: &str = "provider";
    const BASE_URL: &str = "https://api.provider.example/v1";

    #[test]
    fn admission_table_refuses_every_unverified_constructor_input() {
        let cases: [(&str, ProviderConfig, Option<&'static str>, &str); 6] = [
            (
                "wrong settings",
                config("model-v1", "other-settings", BACKEND, BASE_URL),
                Some("remote provider required"),
                "unsupported",
            ),
            (
                "wrong backend",
                config("model-v1", SETTINGS, "other", BASE_URL),
                Some("remote provider required"),
                "unsupported",
            ),
            (
                "lookalike host",
                config(
                    "model-v1",
                    SETTINGS,
                    BACKEND,
                    "https://api.provider.example.evil/v1",
                ),
                Some("remote provider required"),
                "unsupported",
            ),
            (
                "unsupported version",
                config(
                    "model-v1",
                    SETTINGS,
                    BACKEND,
                    "https://api.provider.example/v2",
                ),
                Some("remote provider required"),
                "unsupported",
            ),
            (
                "local configuration with a named refusal",
                local("model-v1", SETTINGS),
                Some("remote provider required"),
                "invalid",
            ),
            (
                "local configuration without a named refusal",
                local("model-v1", SETTINGS),
                None,
                "unsupported",
            ),
        ];
        for (name, provider, local_refusal, expected) in cases {
            let error = admit(&provider, Some(SETTINGS), BACKEND, BASE_URL, local_refusal)
                .expect_err("unverified configuration admitted");
            let matches_expected = match expected {
                "unsupported" => matches!(error, Error::Unsupported),
                _ => matches!(error, Error::Invalid(_)),
            };
            assert!(matches_expected, "{name} returned {error:?}");
        }
        admit(
            &config(
                "model-v1",
                SETTINGS,
                BACKEND,
                "https://api.provider.example/v1/",
            ),
            Some(SETTINGS),
            BACKEND,
            BASE_URL,
            Some("remote provider required"),
        )
        .unwrap();
    }

    #[test]
    fn provider_config_binds_the_selected_model_to_its_settings() {
        let provider = provider_config(
            "model-v1",
            SETTINGS,
            BACKEND,
            BASE_URL,
            "provider",
            "PROVIDER_API_KEY",
        )
        .unwrap();
        assert_eq!(provider.model().as_str(), "model-v1");
        let build = |model: &str, settings: &str| {
            provider_config(
                model,
                settings,
                BACKEND,
                BASE_URL,
                "provider",
                "PROVIDER_API_KEY",
            )
            .unwrap()
        };
        for model in ["x".repeat(256), "λ".repeat(128), "e\u{301}".into()] {
            let config = build(&model, SETTINGS);
            assert_eq!(config.model(), &NfcText::from(model));
            assert!(config.settings_fingerprint().as_str().len() <= 256);
            admit(&config, Some(SETTINGS), BACKEND, BASE_URL, None).unwrap();
        }
        assert_ne!(
            provider.settings_fingerprint(),
            build("model-v1", "changed-settings").settings_fingerprint(),
        );
        assert_ne!(
            build("b;model=c", "a").settings_fingerprint(),
            build("c", "a;model=b").settings_fingerprint(),
            "component framing must preserve the complete settings/model identity",
        );
        let changed = ProviderConfig::new(
            provider.id().clone(),
            "another-model".into(),
            provider.settings_fingerprint().clone(),
            provider.location().clone(),
        )
        .unwrap();
        assert!(matches!(
            admit(&changed, Some(SETTINGS), BACKEND, BASE_URL, None),
            Err(Error::Unsupported)
        ));
        assert!(matches!(
            provider_config(
                "",
                SETTINGS,
                BACKEND,
                BASE_URL,
                "provider",
                "PROVIDER_API_KEY",
            ),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn wire_arithmetic_keeps_encoded_header_and_rounding_rules() {
        let scope = Scope {
            provider: config("model-v1", SETTINGS, BACKEND, BASE_URL),
            operation: Operation::Transcribe,
            payload_fingerprint: "sha256:audio".into(),
            mode: "file".into(),
            texts: Vec::new(),
            source_language: LanguageTag::parse("vi").unwrap(),
            target_language: None,
            source_samples: crate::SampleRange::new(16_000, 48_000).unwrap(),
            sample_rate: 16_000,
            max_bytes: 64_044,
        };
        assert_eq!(duration_micros(&scope).unwrap(), 2_000_000);
        assert_eq!(rounded_up_seconds(&scope), Some(2));
        assert_eq!(encoded_bytes(&scope, 64_044).unwrap(), 64_044);
        assert!(matches!(encoded_bytes(&scope, 64_043), Err(Error::Payload)));

        let mut partial = scope.clone();
        partial.source_samples = crate::SampleRange::new(0, 16_001).unwrap();
        assert_eq!(rounded_up_seconds(&partial), Some(2));
        let mut empty_rate = scope;
        empty_rate.sample_rate = 0;
        assert_eq!(rounded_up_seconds(&empty_rate), None);
    }

    #[test]
    fn status_table_is_shared_and_body_rules_stay_provider_owned() {
        let response = |status, body: &[u8]| HttpResponse {
            status,
            headers: vec![("retry-after".into(), "7".into())],
            body: body.to_vec(),
        };
        assert!(matches!(
            status_error(&response(401, b""), |_, _| None),
            Error::Auth
        ));
        assert!(matches!(
            status_error(&response(413, b""), |_, _| None),
            Error::Payload
        ));
        assert!(matches!(
            status_error(&response(302, b""), |_, _| None),
            Error::Invalid(_)
        ));
        assert!(matches!(
            status_error(&response(429, b""), |_, _| None),
            Error::RateLimit {
                retry_after_seconds: Some(7)
            }
        ));
        assert!(matches!(
            status_error(&response(503, b""), |_, _| None),
            Error::Server
        ));
        assert!(matches!(
            status_error(
                &response(
                    429,
                    br#"{"error":{"code":"insufficient_quota","type":"insufficient_quota"}}"#
                ),
                |status, fields| (status == 429 && quota(fields)).then_some(Error::Quota)
            ),
            Error::Quota
        ));
        assert!(matches!(
            status_error(
                &response(400, br#"{"detail":{"status":"content_policy_violation"}}"#),
                |_, fields| matches(fields, &["content_policy"]).then_some(Error::ContentPolicy)
            ),
            Error::ContentPolicy
        ));
        assert!(!matches(&error_fields(b"not json"), &["quota"]));
    }
}

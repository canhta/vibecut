//! Resolves the selected clip suggestion provider.
use super::{
    ANTHROPIC_SETTINGS, AnthropicClipSuggester, Arc, BuiltSuggester, CatalogOperation,
    ClipDisclosure, ClipInput, ClipSuggester, Consent, Cost, CostEstimate, EditError,
    HeuristicClipSuggester, LanguageTag, Location, Money, OPENAI_LUNA_MODEL, OPENAI_POLICY_URL,
    OPENAI_SETTINGS, OpenAiClipSuggester, PromptApproval, ProviderConfig, Scope, SuggestionOption,
    SuggestionProvider, UploadRequest, engine_providers, subtitles,
};
impl SuggestionProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::OpenAi { .. } => "openai",
            Self::Anthropic { .. } => "anthropic",
        }
    }
    pub fn model(&self) -> &str {
        match self {
            Self::Heuristic => "sentence-pause-keyword",
            Self::OpenAi { model } | Self::Anthropic { model } => model,
        }
    }
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Heuristic)
    }
}
pub fn suggestion_options() -> Vec<SuggestionOption> {
    vec![
        SuggestionOption {
            id: "heuristic",
            label: "Local heuristic (no upload)",
            local: true,
            models: vec!["sentence-pause-keyword"],
            policy_url: None,
        },
        SuggestionOption {
            id: "openai",
            label: "OpenAI (remote)",
            local: false,
            models: vec![OPENAI_LUNA_MODEL, ai::openai::TRANSLATION_MODEL],
            policy_url: Some(OPENAI_POLICY_URL),
        },
        SuggestionOption {
            id: "anthropic",
            label: "Anthropic (remote)",
            local: false,
            models: vec![ai::anthropic::MODEL],
            policy_url: Some(ai::anthropic::POLICY_URL),
        },
    ]
}

pub fn choose_suggestion(
    provider: &str,
    model: Option<&str>,
) -> Result<SuggestionProvider, EditError> {
    let option = suggestion_options()
        .into_iter()
        .find(|option| option.id == provider)
        .ok_or(EditError::Unsupported("unknown clip suggestion provider"))?;
    let model = model
        .or_else(|| option.models.first().copied())
        .ok_or(EditError::Unsupported("missing clip suggestion model"))?;
    if !option.models.contains(&model) {
        return Err(EditError::Unsupported(
            "unsupported model for clip suggestion provider",
        ));
    }
    match option.id {
        "heuristic" => Ok(SuggestionProvider::Heuristic),
        "openai" => Ok(SuggestionProvider::OpenAi {
            model: model.into(),
        }),
        "anthropic" => Ok(SuggestionProvider::Anthropic {
            model: model.into(),
        }),
        _ => Err(EditError::Unsupported("unknown clip suggestion provider")),
    }
}
impl BuiltSuggester {
    pub(super) fn as_clip(&self) -> &dyn ClipSuggester {
        match self {
            Self::Heuristic(value) => value,
            Self::OpenAi(value) => value,
            Self::Anthropic(value) => value,
        }
    }
    pub(super) fn disclosure(
        &self,
        window: &ClipInput,
        scope: &Scope,
    ) -> Result<Option<ClipDisclosure>, EditError> {
        match self {
            Self::Heuristic(_) => Ok(None),
            Self::OpenAi(value) => Ok(Some(value.disclosure(window, scope)?)),
            Self::Anthropic(value) => Ok(Some(value.disclosure(window, scope)?)),
        }
    }
}

pub(super) fn build_suggester(
    provider: &SuggestionProvider,
    credentials: &subtitles::Credentials,
) -> Result<BuiltSuggester, EditError> {
    match provider {
        SuggestionProvider::Heuristic => {
            Ok(BuiltSuggester::Heuristic(HeuristicClipSuggester::new()?))
        }
        SuggestionProvider::OpenAi { model } => {
            let config = ProviderConfig::new(
                "openai".into(),
                model.as_str().into(),
                OPENAI_SETTINGS.into(),
                Location::Remote {
                    backend: ai::openai::BACKEND.into(),
                    base_url: ai::openai::BASE_URL.into(),
                    auth_ref: "OPENAI_API_KEY".into(),
                },
            )?;
            Ok(BuiltSuggester::OpenAi(OpenAiClipSuggester::new(
                config,
                credentials.openai()?.clone(),
                Arc::new(ai::transport::UreqTransport::new()),
            )?))
        }
        SuggestionProvider::Anthropic { model } => {
            let config = ProviderConfig::new(
                "anthropic".into(),
                model.as_str().into(),
                ANTHROPIC_SETTINGS.into(),
                Location::Remote {
                    backend: ai::anthropic::BACKEND.into(),
                    base_url: ai::anthropic::BASE_URL.into(),
                    auth_ref: "ANTHROPIC_API_KEY".into(),
                },
            )?;
            Ok(BuiltSuggester::Anthropic(AnthropicClipSuggester::new(
                config,
                credentials.anthropic()?.clone(),
                Arc::new(ai::transport::UreqTransport::new()),
            )?))
        }
    }
}

pub(super) fn upload_request(
    disclosure: &ClipDisclosure,
    language: &LanguageTag,
) -> Result<UploadRequest, EditError> {
    Ok(UploadRequest {
        operation: CatalogOperation::SuggestClips,
        provider: disclosure.provider.into(),
        model: disclosure.model.as_str().into(),
        languages: language.as_str().into(),
        audio_seconds: 0.0,
        upload_bytes: u64::try_from(disclosure.payload.as_str().len())
            .map_err(|_| EditError::Limit)?,
        cost: Cost::from(&disclosure.estimate),
        terms: vec![
            ("endpoint", disclosure.endpoint.into()),
            (
                "operation",
                "clip suggestions: transcript segments, no audio".into(),
            ),
            ("retention_training_policy", disclosure.policy_url.into()),
        ],
    })
}
impl engine_providers::Approval for PromptApproval<'_> {
    fn approve(
        &mut self,
        scope: &Scope,
        estimate: &CostEstimate,
        _fallback: bool,
    ) -> Result<Option<Consent>, ai::Error> {
        self.spent.add(estimate);
        if !scope.provider.is_remote() {
            return Ok(None);
        }
        let request = self
            .requests
            .iter()
            .find(|(planned, _)| planned == scope)
            .map(|(_, request)| request)
            .ok_or(ai::Error::Invalid("clip consent scope"))?;
        if let Err(error) = self.consent.approve(request) {
            self.refusal = Some(error);
            return Err(ai::Error::ConsentRequired);
        }
        let money = match estimate {
            CostEstimate::Free => Money::new("USD", 0)?,
            CostEstimate::UpperBound(money) => money.clone(),
            CostEstimate::Unknown => return Err(ai::Error::Cost),
        };
        Ok(Some(Consent::grant(scope.clone(), money)?))
    }
}

/// Refuse a remote suggestion choice without its credential before ASR work starts.
pub fn check_suggestion_credentials(
    provider: &SuggestionProvider,
    credentials: &subtitles::Credentials,
) -> Result<(), EditError> {
    match provider {
        SuggestionProvider::Heuristic => Ok(()),
        SuggestionProvider::OpenAi { .. } => credentials.openai().map(|_| ()),
        SuggestionProvider::Anthropic { .. } => credentials.anthropic().map(|_| ()),
    }
}

//! Resolves the selected clip suggestion provider.
use super::{
    AnthropicClipSuggester, Arc, BuiltSuggester, CatalogOperation, ClipSuggester, Cost,
    Credentials, EditError, Error, HeuristicClipSuggester, LanguageTag, OPENAI_LUNA_MODEL,
    OPENAI_POLICY_URL, OpenAiClipSuggester, Scope, SuggestionOption, SuggestionProvider,
    UploadRequest,
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

pub fn choose_suggestion(provider: &str, model: Option<&str>) -> Result<SuggestionProvider, Error> {
    let option = suggestion_options()
        .into_iter()
        .find(|option| option.id == provider)
        .ok_or(EditError::Unsupported("unknown clip suggestion provider"))?;
    let model = model
        .or_else(|| option.models.first().copied())
        .ok_or(EditError::Unsupported("missing clip suggestion model"))?;
    if !option.models.contains(&model) {
        return Err(
            EditError::Unsupported("unsupported model for clip suggestion provider").into(),
        );
    }
    match option.id {
        "heuristic" => Ok(SuggestionProvider::Heuristic),
        "openai" => Ok(SuggestionProvider::OpenAi {
            model: model.into(),
        }),
        "anthropic" => Ok(SuggestionProvider::Anthropic {
            model: model.into(),
        }),
        _ => Err(EditError::Unsupported("unknown clip suggestion provider").into()),
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
        scope: &Scope,
    ) -> Result<Option<ai::UploadDisclosure>, EditError> {
        self.as_clip().disclosure(scope).map_err(Into::into)
    }
}

pub(super) fn build_suggester(
    provider: &SuggestionProvider,
    credentials: &Credentials,
) -> Result<BuiltSuggester, EditError> {
    match provider {
        SuggestionProvider::Heuristic => {
            Ok(BuiltSuggester::Heuristic(HeuristicClipSuggester::new()?))
        }
        SuggestionProvider::OpenAi { model } => {
            Ok(BuiltSuggester::OpenAi(OpenAiClipSuggester::new(
                OpenAiClipSuggester::provider_config(model)?,
                credentials.openai()?.clone(),
                Arc::new(ai::transport::UreqTransport::new()),
            )?))
        }
        SuggestionProvider::Anthropic { model } => {
            Ok(BuiltSuggester::Anthropic(AnthropicClipSuggester::new(
                AnthropicClipSuggester::provider_config(model)?,
                credentials.anthropic()?.clone(),
                Arc::new(ai::transport::UreqTransport::new()),
            )?))
        }
    }
}

pub(super) fn upload_request(
    scope: &Scope,
    disclosure: &ai::UploadDisclosure,
    language: &LanguageTag,
) -> Result<UploadRequest, EditError> {
    let link = |name: &str| -> Result<String, EditError> {
        disclosure
            .link(name)
            .map(|url| url.as_str().into())
            .ok_or(EditError::Unsupported("clip disclosure link"))
    };
    Ok(UploadRequest {
        config: scope.provider.clone(),
        operation: CatalogOperation::SuggestClips,
        provider: disclosure.provider.as_str().into(),
        model: disclosure.model.as_str().into(),
        languages: language.as_str().into(),
        audio_seconds: 0.0,
        upload_bytes: disclosure.upload_bytes,
        cost: Cost::from(&disclosure.estimate),
        terms: vec![
            ("endpoint", link("endpoint")?),
            (
                "operation",
                "clip suggestions: transcript segments, no audio".into(),
            ),
            (
                "retention_training_policy",
                link("retention_training_policy")?,
            ),
        ],
    })
}
/// Refuse a remote suggestion choice without its credential before ASR work starts.
pub fn check_suggestion_credentials(
    provider: &SuggestionProvider,
    credentials: &Credentials,
) -> Result<(), Error> {
    match provider {
        SuggestionProvider::Heuristic => Ok(()),
        SuggestionProvider::OpenAi { .. } => {
            credentials.openai().map(|_| ()).map_err(Error::Engine)
        }
        SuggestionProvider::Anthropic { .. } => {
            credentials.anthropic().map(|_| ()).map_err(Error::Engine)
        }
    }
}

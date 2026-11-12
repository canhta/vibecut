//! Discloses each remote upload before it is approved.
use super::catalog::CatalogOperation;
use super::error::{Error, Result};
use ai::{CostEstimate, Money, ProviderConfig, Scope, UploadDisclosure};
/// A provider disclosure shown before any remote byte leaves the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cost {
    Free,
    UpperBound { currency: String, micros: u64 },
    Unknown,
}
#[derive(Clone, Debug)]
pub struct UploadRequest {
    /// Exact adapter identity, including endpoint, settings and credential reference.
    /// Stored approvals bind this value rather than the display labels below.
    pub config: ProviderConfig,
    pub operation: CatalogOperation,
    pub provider: String,
    pub model: String,
    pub languages: String,
    pub audio_seconds: f64,
    pub upload_bytes: u64,
    pub cost: Cost,
    pub terms: Vec<(&'static str, String)>,
}
impl UploadRequest {
    /// The one builder of upload consent: the adapter's neutral disclosure for
    /// this exact scope. An adapter without a disclosure is a local one, which
    /// has no upload; a remote adapter that supplies none (a composed test
    /// provider) is described from the scope and its configuration alone.
    pub fn for_scope(
        operation: CatalogOperation,
        config: &ProviderConfig,
        disclosure: Option<UploadDisclosure>,
        estimate: &CostEstimate,
        scope: &Scope,
    ) -> Result<Self> {
        if let Some(disclosure) = disclosure {
            return Ok(Self {
                config: config.clone(),
                operation,
                provider: disclosure.provider.as_str().into(),
                model: disclosure.model.as_str().into(),
                languages: disclosure.languages.as_str().into(),
                audio_seconds: disclosure.audio_micros as f64 / 1_000_000.0,
                upload_bytes: disclosure.upload_bytes,
                cost: Cost::from(&disclosure.estimate),
                terms: disclosure
                    .links
                    .iter()
                    .map(|(name, url)| (*name, url.as_str().into()))
                    .collect(),
            });
        }
        if !config.is_remote() {
            return Err(Error::Edit(crate::EditError::Unsupported(
                "local provider has no upload",
            )));
        }
        let languages = match &scope.target_language {
            Some(target) => format!("{} -> {}", scope.source_language.as_str(), target.as_str()),
            None => scope.source_language.as_str().into(),
        };
        let audio_seconds = if scope.texts.is_empty() {
            scope.source_samples.len() as f64 / f64::from(scope.sample_rate)
        } else {
            0.0
        };
        Ok(Self {
            config: config.clone(),
            operation,
            provider: config.id().as_str().into(),
            model: config.model().as_str().into(),
            languages,
            audio_seconds,
            upload_bytes: scope.max_bytes,
            cost: Cost::from(estimate),
            terms: vec![("policy", "test provider policy".into())],
        })
    }
}
/// The adapter refuses the remote request when this returns `Err(ConsentRequired)`.
pub trait ConsentPrompt {
    fn approve(&self, request: &UploadRequest) -> Result<()>;
    /// A separate affirmative decision for the exact displayed model's unknown price.
    /// Ordinary upload approval never implicitly authorizes unknown spending.
    fn approve_unknown_price(&self, _request: &UploadRequest) -> Result<()> {
        Err(ai::Error::Cost.into())
    }
}
/// Show the exact disclosure and grant a payload-bound token only after the
/// appropriate human decision. Unknown pricing never fabricates a money ceiling.
pub fn approve_upload(
    prompt: &dyn ConsentPrompt,
    request: &UploadRequest,
    scope: &Scope,
    estimate: &CostEstimate,
) -> Result<ai::Consent> {
    let operation = match request.operation {
        CatalogOperation::Transcribe => ai::Operation::Transcribe,
        CatalogOperation::Translate => ai::Operation::Translate,
        CatalogOperation::SuggestClips => ai::Operation::SuggestClips,
        CatalogOperation::GenerateChapters => ai::Operation::GenerateChapters,
        CatalogOperation::ClassifyFillers => ai::Operation::ClassifyFillers,
        CatalogOperation::SelectEmphasis => ai::Operation::SelectEmphasis,
    };
    if request.config != scope.provider
        || operation != scope.operation
        || request.model != scope.provider.model().as_str()
        || request.cost != Cost::from(estimate)
    {
        return Err(ai::Error::ConsentRequired.into());
    }
    if matches!(estimate, CostEstimate::Unknown) {
        prompt.approve_unknown_price(request)?;
        let acknowledgement =
            ai::UnknownPriceAcknowledgement::grant(scope.provider.clone(), scope.operation);
        Ok(ai::Consent::grant_unknown_price(
            scope.clone(),
            &acknowledgement,
        )?)
    } else {
        prompt.approve(request)?;
        Ok(ai::Consent::grant(
            scope.clone(),
            estimate_money(estimate)?,
        )?)
    }
}
/// A prompt that refuses every remote request. Test fixtures and non-interactive
/// callers must opt in explicitly with [`AllowAll`].
pub struct DenyUploads;
pub struct AllowAll;
impl Cost {
    /// Accumulate an estimate, saturating micros and marking mixed currencies unknown.
    pub fn add(&mut self, next: &CostEstimate) {
        match (&mut *self, next) {
            (Self::Unknown, _) => {}
            (_, CostEstimate::Unknown) => *self = Self::Unknown,
            (Self::Free, CostEstimate::Free) => {}
            (Self::Free, CostEstimate::UpperBound(money)) => {
                *self = Self::UpperBound {
                    currency: money.currency().into(),
                    micros: money.micros(),
                };
            }
            (Self::UpperBound { currency, micros }, CostEstimate::Free) => {
                let _ = (currency, micros);
            }
            (Self::UpperBound { currency, micros }, CostEstimate::UpperBound(money))
                if currency == money.currency() =>
            {
                *micros = micros.saturating_add(money.micros());
            }
            (Self::UpperBound { .. }, CostEstimate::UpperBound(_)) => *self = Self::Unknown,
        }
    }
}
impl From<&CostEstimate> for Cost {
    fn from(value: &CostEstimate) -> Self {
        match value {
            CostEstimate::Free => Self::Free,
            CostEstimate::UpperBound(money) => Self::UpperBound {
                currency: money.currency().into(),
                micros: money.micros(),
            },
            CostEstimate::Unknown => Self::Unknown,
        }
    }
}
impl ConsentPrompt for DenyUploads {
    fn approve(&self, _: &UploadRequest) -> Result<()> {
        Err(Error::ConsentRequired)
    }
}
impl ConsentPrompt for AllowAll {
    fn approve(&self, _: &UploadRequest) -> Result<()> {
        Ok(())
    }
}
pub(crate) fn estimate_money(estimate: &CostEstimate) -> Result<Money> {
    match estimate {
        CostEstimate::Free => Ok(Money::new("USD", 0)?),
        CostEstimate::UpperBound(money) => Ok(money.clone()),
        CostEstimate::Unknown => Err(Error::Ai(ai::Error::Cost)),
    }
}

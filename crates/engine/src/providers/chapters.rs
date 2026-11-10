//! Lends the selected chapter adapter and its human consent to one job.
//!
//! The composition root chooses the provider and supplies the approval callback;
//! the access binds the exact input, cost and provider configuration into the
//! token the adapter receives. Without an affirmative approval it refuses before
//! any adapter or credential is used.
use super::adapters::{Credentials, open_chapter};
use super::catalog::TextProvider;
use super::selection::Approval;
use crate::EditError;
use ai::chapters::{ChapterGenerator, ChapterInput, MAX_WIRE, MODE};
use ai::{Consent, CostEstimate, Money, Operation, ProviderConfig, SampleRange, Scope};
use captions::NfcText;
use std::sync::{Arc, Mutex};

/// One process's selected chapter adapter and the human approval it consults.
pub struct ChapterAccess {
    generator: Arc<dyn ChapterGenerator>,
    approval: Mutex<Box<dyn Approval + Send>>,
}

/// The consent token and adapter one validated input runs with.
pub(crate) struct PreparedChapter {
    pub(crate) generator: Arc<dyn ChapterGenerator>,
    pub(crate) scope: Scope,
    pub(crate) consent: Consent,
}

/// A human consent the composition root recorded for one exact provider
/// configuration and cost ceiling. It never prompts and never broadens: a
/// different provider or a higher estimate is refused by the token check.
pub struct StoredConsent {
    provider: ProviderConfig,
    budget: Money,
}
impl StoredConsent {
    /// Record the human's affirmative approval after showing this provider and cost.
    pub fn grant(provider: ProviderConfig, budget: Money) -> Self {
        Self { provider, budget }
    }
}
impl Approval for StoredConsent {
    fn approve(
        &mut self,
        scope: &Scope,
        _: &CostEstimate,
        _: bool,
    ) -> Result<Option<Consent>, ai::Error> {
        if scope.provider != self.provider {
            return Ok(None);
        }
        Ok(Some(Consent::grant(scope.clone(), self.budget.clone())?))
    }
}

impl ChapterAccess {
    /// Lend an already built adapter, for composition roots and tests.
    pub fn new(generator: Arc<dyn ChapterGenerator>, approval: Box<dyn Approval + Send>) -> Self {
        Self {
            generator,
            approval: Mutex::new(approval),
        }
    }

    /// Build the selected provider's adapter and lend it with `approval`.
    pub fn open(
        provider: TextProvider,
        credentials: &Credentials,
        approval: Box<dyn Approval + Send>,
    ) -> Result<Self, EditError> {
        Ok(Self::new(open_chapter(&provider, credentials)?, approval))
    }

    /// The exact provider configuration every request is bound to.
    pub fn provider(&self) -> &ProviderConfig {
        self.generator.config()
    }

    /// Bind the complete input to a fresh consent token; no token, no adapter call.
    pub(crate) fn prepare(&self, input: &ChapterInput) -> Result<PreparedChapter, EditError> {
        let payload = input.consent_text()?;
        let bytes = u64::try_from(payload.as_str().len()).map_err(|_| EditError::Limit)?;
        let fingerprint: NfcText = blake3::hash(payload.as_str().as_bytes())
            .to_hex()
            .to_string()
            .into();
        let scope = Scope {
            provider: self.generator.config().clone(),
            operation: Operation::GenerateChapters,
            payload_fingerprint: fingerprint,
            mode: MODE.into(),
            texts: vec![payload],
            source_language: input.settings.language.clone(),
            target_language: None,
            source_samples: SampleRange::new(0, 1)?,
            sample_rate: 1,
            max_bytes: bytes.max(MAX_WIRE),
        };
        let estimate = self.generator.estimate(&scope);
        let consent = self
            .approval
            .lock()
            .map_err(|_| EditError::JobPoisoned)?
            .approve(&scope, &estimate, false)?
            .ok_or(EditError::ConsentRequired)?;
        Ok(PreparedChapter {
            generator: self.generator.clone(),
            scope,
            consent,
        })
    }
}

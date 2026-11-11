//! Lends the selected transcript-text adapters and their human consent to one job.
//!
//! Chapter generation and filler classification share one approved text provider,
//! one approval callback and one consent token contract. The composition root
//! chooses the provider and supplies the approval; the access binds the exact
//! input, cost and provider configuration into the token an adapter receives.
//! Without an affirmative approval it refuses before any adapter or credential
//! is used.
use super::adapters::{Credentials, open_chapter, open_filler};
use super::catalog::TextProvider;
use super::error::Error;
use super::selection::Approval;
use crate::EditError;
use ai::chapters::{ChapterGenerator, ChapterInput};
use ai::fillers::{FillerClassifier, FillerInput};
use ai::{Consent, CostEstimate, Money, Operation, ProviderConfig, SampleRange, Scope};
use captions::{LanguageTag, NfcText};
use std::sync::{Arc, Mutex};

/// One process's selected transcript-text adapters and the human approval they consult.
pub struct TextAccess {
    chapter: Arc<dyn ChapterGenerator>,
    filler: Arc<dyn FillerClassifier>,
    approval: Mutex<Box<dyn Approval + Send>>,
}

/// The consent token and chapter adapter one validated input runs with.
pub(crate) struct PreparedChapter {
    pub(crate) generator: Arc<dyn ChapterGenerator>,
    pub(crate) scope: Scope,
    pub(crate) consent: Consent,
}
/// The consent token and filler adapter one validated input runs with.
pub(crate) struct PreparedFillers {
    pub(crate) classifier: Arc<dyn FillerClassifier>,
    pub(crate) scope: Scope,
    pub(crate) consent: Consent,
}

/// A human consent the composition root recorded for each exact provider
/// configuration and cost ceiling. It never prompts and never broadens: a
/// different provider or a higher estimate is refused by the token check.
pub struct StoredConsent {
    grants: Vec<(ProviderConfig, Money)>,
}
impl StoredConsent {
    /// Record the human's affirmative approvals after showing each provider and cost.
    pub fn grant(grants: impl IntoIterator<Item = (ProviderConfig, Money)>) -> Self {
        Self {
            grants: grants.into_iter().collect(),
        }
    }
}
impl Approval for StoredConsent {
    fn approve(
        &mut self,
        scope: &Scope,
        _: &CostEstimate,
        _: bool,
    ) -> Result<Option<Consent>, ai::Error> {
        self.grants
            .iter()
            .find(|(provider, _)| provider == &scope.provider)
            .map(|(_, budget)| Consent::grant(scope.clone(), budget.clone()))
            .transpose()
    }
}

impl TextAccess {
    /// Lend already built adapters, for composition roots and tests. One approval
    /// covers both operations because they use the same provider configuration.
    pub fn new(
        chapter: Arc<dyn ChapterGenerator>,
        filler: Arc<dyn FillerClassifier>,
        approval: Box<dyn Approval + Send>,
    ) -> Self {
        Self {
            chapter,
            filler,
            approval: Mutex::new(approval),
        }
    }

    /// Build the selected provider's adapters and lend them with `approval`.
    pub fn open(
        provider: TextProvider,
        credentials: &Credentials,
        approval: Box<dyn Approval + Send>,
    ) -> Result<Self, EditError> {
        Ok(Self::new(
            open_chapter(&provider, credentials)?,
            open_filler(&provider, credentials)?,
            approval,
        ))
    }

    /// The exact provider configuration chapter requests are bound to.
    pub fn chapter_provider(&self) -> &ProviderConfig {
        self.chapter.config()
    }

    /// The exact provider configuration filler requests are bound to.
    pub fn filler_provider(&self) -> &ProviderConfig {
        self.filler.config()
    }

    /// Bind the complete chapter input to a fresh consent token; no token, no adapter call.
    pub(crate) fn prepare_chapter(
        &self,
        input: &ChapterInput,
    ) -> Result<PreparedChapter, EditError> {
        let scope = self.scope(
            Operation::GenerateChapters,
            ai::chapters::MODE,
            self.chapter_provider(),
            input.consent_text()?,
            &input.settings.language,
            ai::chapters::MAX_WIRE,
        )?;
        let estimate = self.chapter.estimate(&scope);
        let consent = self.approve(&scope, &estimate)?;
        Ok(PreparedChapter {
            generator: self.chapter.clone(),
            scope,
            consent,
        })
    }

    /// Bind the complete filler transcript to a fresh consent token; no token, no adapter call.
    pub(crate) fn prepare_fillers(
        &self,
        input: &FillerInput,
    ) -> Result<PreparedFillers, EditError> {
        let scope = self.scope(
            Operation::ClassifyFillers,
            ai::fillers::MODE,
            self.filler_provider(),
            input.consent_text()?,
            &input.settings.language,
            ai::fillers::MAX_WIRE,
        )?;
        let estimate = self.filler.estimate(&scope);
        let consent = self.approve(&scope, &estimate)?;
        Ok(PreparedFillers {
            classifier: self.filler.clone(),
            scope,
            consent,
        })
    }

    fn scope(
        &self,
        operation: Operation,
        mode: &str,
        provider: &ProviderConfig,
        payload: NfcText,
        language: &LanguageTag,
        max_wire: u64,
    ) -> Result<Scope, EditError> {
        let bytes = u64::try_from(payload.as_str().len()).map_err(|_| EditError::Limit)?;
        let fingerprint: NfcText = blake3::hash(payload.as_str().as_bytes())
            .to_hex()
            .to_string()
            .into();
        Ok(Scope {
            provider: provider.clone(),
            operation,
            payload_fingerprint: fingerprint,
            mode: mode.into(),
            texts: vec![payload],
            source_language: language.clone(),
            target_language: None,
            source_samples: SampleRange::new(0, 1)?,
            sample_rate: 1,
            max_bytes: bytes.max(max_wire),
        })
    }

    fn approve(&self, scope: &Scope, estimate: &CostEstimate) -> Result<Consent, EditError> {
        Ok(self
            .approval
            .lock()
            .map_err(|_| crate::RuntimeError::Poisoned)?
            .approve(scope, estimate, false)?
            .ok_or(Error::ConsentRequired)?)
    }
}

//! Lends the selected transcript-text adapters and their human consent to one job.
//!
//! Chapter generation and filler classification share one approved text provider,
//! one approval callback and one consent token contract. The composition root
//! chooses the provider and supplies the approval; the access binds the exact
//! input, cost and provider configuration into the token an adapter receives.
//! Without an affirmative approval it refuses before any adapter or credential
//! is used.
use super::adapters::{Credentials, open_chapter, open_filler};
use super::catalog::{CatalogOperation, TextProvider};
use super::consent::{ConsentPrompt, Cost, UploadRequest, estimate_money};
use super::error::Error;
use crate::EditError;
use ai::chapters::{ChapterGenerator, ChapterInput};
use ai::fillers::{FillerClassifier, FillerInput};
use ai::{Consent, CostEstimate, Money, ProviderConfig, Scope, UploadDisclosure};
use captions::NfcText;
use std::sync::Arc;

/// One process's selected transcript-text adapters and the human approval they consult.
pub struct TextAccess {
    chapter: Arc<dyn ChapterGenerator>,
    filler: Arc<dyn FillerClassifier>,
    consent: Box<dyn ConsentPrompt + Send + Sync>,
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
/// configuration and cost ceiling. It never prompts or broadens the approval.
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
impl ConsentPrompt for StoredConsent {
    fn approve(&self, request: &UploadRequest) -> Result<(), EditError> {
        let covered = |ceiling: &Money| match &request.cost {
            Cost::Free => true,
            Cost::UpperBound { currency, micros } => {
                currency == ceiling.currency() && *micros <= ceiling.micros()
            }
            Cost::Unknown => false,
        };
        let (_, ceiling) = self
            .grants
            .iter()
            .find(|(config, _)| config == &request.config)
            .ok_or(Error::ConsentRequired)?;
        if covered(ceiling) {
            Ok(())
        } else {
            Err(ai::Error::Cost.into())
        }
    }
}

impl TextAccess {
    /// Lend already built adapters, for composition roots and tests. One consent
    /// covers both operations because they use the same provider configuration.
    pub fn new(
        chapter: Arc<dyn ChapterGenerator>,
        filler: Arc<dyn FillerClassifier>,
        consent: Box<dyn ConsentPrompt + Send + Sync>,
    ) -> Self {
        Self {
            chapter,
            filler,
            consent,
        }
    }

    /// Build the selected provider's adapters and lend them with `consent`.
    pub fn open(
        provider: TextProvider,
        credentials: &Credentials,
        consent: Box<dyn ConsentPrompt + Send + Sync>,
    ) -> Result<Self, EditError> {
        Ok(Self::new(
            open_chapter(&provider, credentials)?,
            open_filler(&provider, credentials)?,
            consent,
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
        let scope = Scope::chapters(
            self.chapter_provider().clone(),
            input,
            fingerprint(&input.consent_text()?),
        )?;
        let estimate = self.chapter.estimate(&scope);
        let consent = self.approve(
            CatalogOperation::GenerateChapters,
            &scope,
            self.chapter.disclosure(&scope)?,
            &estimate,
        )?;
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
        let scope = Scope::fillers(
            self.filler_provider().clone(),
            input,
            fingerprint(&input.consent_text()?),
        )?;
        let estimate = self.filler.estimate(&scope);
        let consent = self.approve(
            CatalogOperation::ClassifyFillers,
            &scope,
            self.filler.disclosure(&scope)?,
            &estimate,
        )?;
        Ok(PreparedFillers {
            classifier: self.filler.clone(),
            scope,
            consent,
        })
    }

    fn approve(
        &self,
        operation: CatalogOperation,
        scope: &Scope,
        disclosure: Option<UploadDisclosure>,
        estimate: &CostEstimate,
    ) -> Result<Consent, EditError> {
        let request =
            UploadRequest::for_scope(operation, &scope.provider, disclosure, estimate, scope)?;
        self.consent.approve(&request)?;
        Ok(Consent::grant(scope.clone(), estimate_money(estimate)?)?)
    }
}

fn fingerprint(payload: &NfcText) -> NfcText {
    blake3::hash(payload.as_str().as_bytes())
        .to_hex()
        .to_string()
        .into()
}

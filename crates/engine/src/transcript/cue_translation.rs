//! Translates a cue set through the chosen translation provider.
use super::checkpoints::wait_for_job;
use super::cues::{build_session, caption_item_id};
use super::operation::{Cancellation, Progress, Stage};
use super::translation::{TranslationRun, TranslationTarget, window};
use super::translation_reference;
use crate::jobs::JobRunner;
use crate::providers::{
    CatalogOperation, ConsentPrompt, Cost, Credentials, Providers, TranslationProvider,
    UploadRequest, approve_upload, build_translator, estimate_money,
};
use crate::{Command, EditError, Session};
use ai::Money;
use captions::LanguageTag;
use captions::{Cue, CueTrack, NfcText};
use media::SourceAsset;
use project::ItemParams;
use std::sync::Arc;
use time::Tick;
pub const TRANSLATE_ATTEMPTS: u8 = 3;
/// A translation-only adapter. Opening it never loads ASR weights or downloads files.
pub struct SubtitleTranslator {
    translator: Arc<dyn ai::Translator>,
    mode: NfcText,
}
/// Exact cue selection and glossary bound into every remote request.
pub struct TranslationSelection {
    pub item: project::ItemId,
    pub ids: Vec<NfcText>,
    pub target: LanguageTag,
    pub glossary: Vec<ai::GlossaryTerm>,
    pub force: bool,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn translate_cues(
    session: &mut Session,
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    _source: &LanguageTag,
    target: &LanguageTag,
    _fingerprint: &NfcText,
    item: project::ItemId,
    cost: &mut Cost,
) -> Result<(), EditError> {
    let ids: Vec<NfcText> = {
        let project = session.project();
        let item = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|value| value.id == item)
            .ok_or(EditError::NoItem(item))?;
        let ItemParams::Caption(params) = &item.content else {
            return Err(EditError::Invalid("translation requires caption item"));
        };
        params.cues.cues.iter().map(|cue| cue.id.clone()).collect()
    };
    let adapter = SubtitleTranslator {
        translator: providers
            .translator
            .clone()
            .ok_or(EditError::Unsupported("missing translator"))?,
        mode: providers
            .translation_mode
            .clone()
            .ok_or(EditError::Unsupported("missing translation mode"))?,
    };
    translate_selected(
        session,
        &adapter,
        runner,
        consent,
        progress,
        cancel,
        &TranslationSelection {
            item,
            ids,
            target: target.clone(),
            glossary: Vec::new(),
            force: false,
        },
        cost,
    )
}
impl SubtitleTranslator {
    pub fn open(
        provider: &TranslationProvider,
        credentials: &Credentials,
    ) -> Result<Self, EditError> {
        let (translator, mode) = build_translator(provider, credentials)?;
        Ok(Self { translator, mode })
    }
    /// Inject an existing adapter while preserving the real translation/consent job path.
    pub fn from_parts(provider: Arc<dyn ai::Translator>, mode: NfcText) -> Self {
        Self {
            translator: provider,
            mode,
        }
    }
}
/// Translate selected cues, preserving manual edits unless force is explicit.
/// Applies through TranslationRun; callers staging a UI batch use a snapshot
/// Session and apply its final command against the original job target.
#[allow(clippy::too_many_arguments)]
pub fn translate_selected(
    session: &mut Session,
    adapter: &SubtitleTranslator,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    selection: &TranslationSelection,
    cost: &mut Cost,
) -> Result<(), EditError> {
    let item = selection.item;
    if selection.ids.is_empty()
        || selection.ids.len() > crate::session::wire::MAX_TRANSLATION_CUES
        || selection
            .ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != selection.ids.len()
    {
        return Err(EditError::Limit);
    }
    progress.stage(Stage::Translate);
    let translator = adapter.translator.as_ref();
    for chunk in selection.ids.chunks(ai::MAX_SEGMENTS.min(64)) {
        cancel.checkpoint()?;
        let window = window(
            session,
            item,
            chunk,
            selection.target.clone(),
            selection.glossary.clone(),
            "subtitle".into(),
            selection.force,
        )?;
        let payload = window.consent_text()?;
        let scope = ai::Scope::translation(
            translator.config().clone(),
            adapter.mode.clone(),
            &window,
            blake3::hash(payload.as_str().as_bytes())
                .to_hex()
                .as_str()
                .into(),
        )?;
        let estimate = translator.estimate(&scope);
        cost.add(&estimate);
        let request = UploadRequest::for_scope(
            CatalogOperation::Translate,
            translator.config(),
            translator.disclosure(&scope)?,
            &estimate,
            &scope,
        )?;
        let consent_token = Some(approve_upload(consent, &request, &scope, &estimate)?);
        let run = TranslationRun::new(
            TranslationTarget { session, item },
            adapter.translator.clone(),
            window,
            scope,
            consent_token,
            TRANSLATE_ATTEMPTS,
            {
                if matches!(estimate, ai::CostEstimate::Unknown) {
                    None
                } else {
                    let each = estimate_money(&estimate)?;
                    Some(Money::new(
                        each.currency(),
                        each.micros().saturating_mul(u64::from(TRANSLATE_ATTEMPTS)),
                    )?)
                }
            },
        )?;
        let (id, token) = run.submit(runner, session)?;
        let output = wait_for_job(runner, id, &token, progress, cancel)?;
        cancel.checkpoint()?;
        let translation: ai::Translation = serde_json::from_slice(output.bytes())
            .map_err(|_| EditError::Invalid("translation response is malformed"))?;
        output.apply(
            session,
            Command::Retranslate {
                item,
                target_language: selection.target.clone(),
                texts: translation.texts,
                translation_ref: translation_reference(translator.config())?,
                force: selection.force,
            },
        )?;
    }
    Ok(())
}

/// Translate one source-timed cue set into a target language without writing a
/// project. Reuses the subtitle translation window/consent/job path.
/// Duration is in source ticks; missing providers, invalid cues, refused consent
/// and cooperative cancellation retain their existing typed execution failures.
#[allow(clippy::too_many_arguments)]
pub fn translate_track(
    asset: &SourceAsset,
    duration: Tick,
    cues: &[Cue],
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    source: &LanguageTag,
    target: &LanguageTag,
    fingerprint: &NfcText,
    cost: &mut Cost,
) -> Result<CueTrack, EditError> {
    if providers.translator().is_none() {
        return Err(EditError::Unsupported("translation provider is required"));
    }
    let mut session = build_session(asset, cues, duration, false, None)?;
    let item = caption_item_id(&session)?;
    progress.stage(Stage::Translate);
    translate_cues(
        &mut session,
        providers,
        runner,
        consent,
        progress,
        cancel,
        source,
        target,
        fingerprint,
        item,
        cost,
    )?;
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find_map(|item| match &item.content {
            ItemParams::Caption(params) => Some(params.cues.clone()),
            _ => None,
        })
        .ok_or(EditError::Invalid("missing translated cues"))
}

//! Submits caption translation as one bounded workflow job.
use super::{
    CaptionAccess,
    outputs::{MAX_CUES, MAX_RESULT_BYTES},
};
use crate::jobs::{JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass};
use crate::providers::{CatalogOperation, Cost, UploadRequest, estimate_money};
use crate::session::wire::{
    API_VERSION, MAX_FRAME_BYTES, MAX_KEY_BYTES, Request, parse_frame, registry_hash,
};
use crate::transcript::{
    TRANSLATE_ATTEMPTS,
    translation::{TranslationRun, TranslationTarget, window},
};
use crate::{Command, EditError, Result, Session, session::wire::TranslationsResult};
use captions::{LanguageTag, NfcText};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn translate(
    session: &Session,
    runner: &mut JobRunner,
    access: &CaptionAccess,
    item: project::ItemId,
    ids: Option<Vec<NfcText>>,
    target: LanguageTag,
    force: bool,
    input: crate::cache::CacheKey,
) -> Result<JobId> {
    let caption = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .ok_or(EditError::NoItem(item))?;
    let project::ItemParams::Caption(params) = &caption.content else {
        return Err(EditError::Invalid("translation requires caption item"));
    };
    let ids = ids.unwrap_or_else(|| params.cues.cues.iter().map(|c| c.id.clone()).collect());
    if ids.is_empty() || ids.len() > MAX_CUES {
        return Err(EditError::Limit);
    }
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(EditError::Invalid("duplicate translation selection"));
    }
    if ids
        .iter()
        .any(|id| !params.cues.cues.iter().any(|c| &c.id == id))
    {
        return Err(EditError::Invalid("missing selected cue"));
    }
    let eligible: Vec<_> = ids
        .into_iter()
        .filter(|id| {
            force
                || !params
                    .cues
                    .cues
                    .iter()
                    .any(|c| &c.id == id && c.texts.get(&target).is_some_and(|t| t.edited))
        })
        .collect();
    if eligible.is_empty() {
        return Err(EditError::Invalid("no unprotected translation selection"));
    }
    let windows = eligible
        .chunks(ai::MAX_SEGMENTS.min(64))
        .map(|ids| {
            window(
                session,
                item,
                ids,
                target.clone(),
                Vec::new(),
                "subtitle".into(),
                force,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let (providers, consent) = access.share()?;
    let translator = providers
        .translator
        .clone()
        .ok_or(EditError::Unsupported("missing translator"))?;
    let mode = providers
        .translation_mode
        .clone()
        .ok_or(EditError::Unsupported("missing translation mode"))?;
    let project = session.project().as_ref().clone();
    let session_id = session.session_id().to_owned();
    let revision = session.revision();
    let attempts = AtomicU8::new(0);
    let cost = Mutex::new(Cost::Free);
    let (id, _) = runner.submit_for_session(
        session,
        JobSpec {
            kind: JobKind::Translate,
            class: ResourceClass::Network,
            input,
            bytes: JobBytes {
                compositor: crate::transcript::WORK_BYTES,
                result: MAX_RESULT_BYTES as u64,
                ..Default::default()
            },
        },
        move |hooks| {
            hooks.check_cancelled()?;
            attempts
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < TRANSLATE_ATTEMPTS).then(|| n + 1)
                })
                .map_err(|_| ai::Error::Cost)?;
            let mut staging = Session::new(project.clone(), 1)?;
            let mut texts = BTreeMap::new();
            for window in &windows {
                hooks.check_cancelled()?;
                let payload = window.consent_text()?;
                let scope = ai::Scope::translation(
                    translator.config().clone(),
                    mode.clone(),
                    window,
                    blake3::hash(payload.as_str().as_bytes())
                        .to_hex()
                        .as_str()
                        .into(),
                )?;
                let estimate = translator.estimate(&scope);
                let request = UploadRequest::for_scope(
                    CatalogOperation::Translate,
                    translator.config(),
                    translator.disclosure(&scope)?,
                    &estimate,
                    &scope,
                )?;
                consent.approve(&request)?;
                let money = estimate_money(&estimate)?;
                let token = ai::Consent::grant(scope.clone(), money.clone())?;
                let run = TranslationRun::new(
                    TranslationTarget {
                        session: &staging,
                        item,
                    },
                    translator.clone(),
                    window.clone(),
                    scope,
                    Some(token),
                    1,
                    money,
                )?;
                cost.lock()
                    .map_err(|_| crate::RuntimeError::Poisoned)?
                    .add(&estimate);
                let bytes = run.run(hooks)?;
                let translated: ai::Translation = serde_json::from_slice(&bytes)
                    .map_err(|_| EditError::Invalid("translation response is malformed"))?;
                staging.apply(Command::Retranslate {
                    item,
                    target_language: target.clone(),
                    texts: translated.texts.clone(),
                    translation_ref: format!(
                        "{}/{}",
                        translator.config().id().as_str(),
                        translator.config().model().as_str()
                    )
                    .into(),
                    force,
                })?;
                texts.extend(translated.texts);
            }
            hooks.check_cancelled()?;
            let config = translator.config();
            let result = TranslationsResult {
                item,
                target_language: target.clone(),
                texts,
                translation_ref: format!("{}/{}", config.id().as_str(), config.model().as_str())
                    .into(),
                cost: (&*cost.lock().map_err(|_| crate::RuntimeError::Poisoned)?).into(),
                provider: config.id().clone(),
                model: config.model().clone(),
            };
            // Successful output must fit one ordinary retranslate request, not
            // just the job-result ceiling. Use the actual admission parser so
            // keys, escaping and the complete envelope count against its limits.
            let apply = Request {
                api_version: API_VERSION,
                registry_hash: Some(registry_hash().into()),
                session_id: session_id.clone(),
                request_id: "r".repeat(MAX_KEY_BYTES),
                idempotency_key: Some("k".repeat(MAX_KEY_BYTES)),
                base_rev: Some(revision.clone()),
                command: "retranslate".into(),
                params: serde_json::json!({
                    "item": result.item,
                    "target_language": result.target_language,
                    "texts": result.texts,
                    "translation_ref": result.translation_ref,
                    "force": force,
                }),
            };
            crate::session::budget(&apply.params)?;
            let frame = crate::session::wire::bounded_json(&apply, MAX_FRAME_BYTES)?;
            parse_frame(&frame).map_err(|_| EditError::Limit)?;
            crate::session::wire::bounded_json(&result, MAX_RESULT_BYTES)
        },
    )?;
    Ok(id)
}

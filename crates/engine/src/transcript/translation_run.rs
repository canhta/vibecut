//! Applies selected translations through bounded attempts.
use crate::{
    EditError, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, ResourceClass},
};
use ai::{
    Consent, CostEstimate, ExecutionHooks, Money, Request, Scope, TranslationWindow, Translator,
};
use captions::{LanguageTag, NfcText};
use project::{ItemId, ItemParams};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

/// Select only the requested source cues, skipping manually edited target texts
/// unless force is explicit. Before/after context has no output IDs.
pub fn window(
    session: &Session,
    item: ItemId,
    selected: &[NfcText],
    target: LanguageTag,
    glossary: Vec<ai::GlossaryTerm>,
    style: NfcText,
    force: bool,
) -> crate::Result<TranslationWindow> {
    if selected.is_empty() || selected.len() > ai::MAX_SEGMENTS {
        return Err(EditError::Limit);
    }
    let ids: BTreeSet<_> = selected.iter().collect();
    if ids.len() != selected.len() {
        return Err(EditError::Invalid("duplicate translation selection"));
    }
    let item = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .ok_or(EditError::NoItem(item))?;
    let ItemParams::Caption(params) = &item.content else {
        return Err(EditError::Invalid("translation requires caption item"));
    };
    let cues = &params.cues.cues;
    if ids.iter().any(|id| !cues.iter().any(|c| &c.id == *id)) {
        return Err(EditError::Invalid("missing selected cue"));
    }
    let mut segments = Vec::new();
    let mut first = None;
    let mut last = 0;
    let mut source = None;
    for (index, cue) in cues.iter().enumerate() {
        if !ids.contains(&cue.id) || (!force && cue.texts.get(&target).is_some_and(|t| t.edited)) {
            continue;
        }
        if source.as_ref().is_some_and(|s| s != &cue.source_lang) {
            return Err(EditError::Invalid("mixed translation source languages"));
        }
        source = Some(cue.source_lang.clone());
        let text = cue
            .texts
            .get(&cue.source_lang)
            .ok_or(EditError::Invalid("missing source text"))?
            .text
            .clone();
        segments.push(ai::TranslationSegment {
            id: cue.id.clone(),
            text,
        });
        first.get_or_insert(index);
        last = index;
    }
    let first = first.ok_or(EditError::Invalid("no unprotected translation selection"))?;
    let source = source.ok_or(EditError::Invalid("translation source"))?;
    let context = |index: usize| {
        cues.get(index)
            .filter(|c| c.source_lang == source)
            .and_then(|c| c.texts.get(&source))
            .map(|t| t.text.clone())
    };
    let before = (first.saturating_sub(2)..first)
        .filter_map(context)
        .collect();
    let after = (last.saturating_add(1)..last.saturating_add(3).min(cues.len()))
        .filter_map(context)
        .collect();
    let window = TranslationWindow {
        source_language: source,
        target_language: target,
        segments,
        before,
        after,
        glossary,
        style,
    };
    window.consent_text()?;
    Ok(window)
}
/// Caption source and Session revision selected before provider work is prepared.
pub struct TranslationTarget<'a> {
    pub session: &'a Session,
    pub item: ItemId,
}
/// One operation-wide ceiling, shared across schema repairs and network retries.
/// Repeated job closure invocations cannot reset the consent/cost attempt budget.
pub struct TranslationRun {
    target: crate::JobTarget,
    provider: Arc<dyn Translator>,
    window: TranslationWindow,
    scope: Scope,
    consent: Option<Consent>,
    attempts: AtomicU8,
    max_attempts: u8,
}
impl TranslationRun {
    pub fn new(
        target: TranslationTarget<'_>,
        provider: Arc<dyn Translator>,
        window: TranslationWindow,
        scope: Scope,
        consent: Option<Consent>,
        max_attempts: u8,
        budget: Option<Money>,
    ) -> crate::Result<Arc<Self>> {
        window.validate()?;
        if !(1..=10).contains(&max_attempts) {
            return Err(EditError::Limit);
        }
        let item = target
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == target.item)
            .ok_or(EditError::NoItem(target.item))?;
        let ItemParams::Caption(params) = &item.content else {
            return Err(EditError::Invalid("translation requires caption item"));
        };
        if window.segments.iter().any(|segment| {
            !params.cues.cues.iter().any(|cue| {
                cue.id == segment.id
                    && cue.source_lang == window.source_language
                    && cue
                        .texts
                        .get(&cue.source_lang)
                        .is_some_and(|text| text.text == segment.text)
            })
        }) {
            return Err(EditError::StaleResult);
        }
        let payload = window.consent_text()?;
        if scope.texts != [payload]
            || scope.source_language != window.source_language
            || scope.target_language.as_ref() != Some(&window.target_language)
            || provider.config() != &scope.provider
        {
            return Err(ai::Error::ConsentRequired.into());
        }
        provider.capabilities(&scope).check(&scope)?;
        let estimate = provider.estimate(&scope);
        ai::authorize(&scope, &estimate, consent.as_ref())?;
        match estimate {
            CostEstimate::Free => {}
            CostEstimate::UpperBound(cost)
                if budget.as_ref().is_some_and(|budget| {
                    cost.currency() == budget.currency()
                        && cost
                            .micros()
                            .checked_mul(u64::from(max_attempts))
                            .is_some_and(|total| total <= budget.micros())
                }) => {}
            CostEstimate::Unknown if budget.is_none() => {}
            _ => return Err(ai::Error::Cost.into()),
        }
        Ok(Arc::new(Self {
            target: target.session.job_target(),
            provider,
            window,
            scope,
            consent,
            attempts: AtomicU8::new(0),
            max_attempts,
        }))
    }
    /// The shared runner calls this once per attempt. Refusal/auth/quota/cost never
    /// retry; Schema and transient transport errors use its bounded backoff policy.
    pub fn run(&self, hooks: &dyn ExecutionHooks) -> crate::Result<Vec<u8>> {
        if hooks.cancelled() {
            return Err(EditError::Cancelled);
        }
        self.attempts
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max_attempts).then(|| n + 1)
            })
            .map_err(|_| ai::Error::Cost)?;
        let output = ai::translate(
            self.provider.as_ref(),
            &self.window,
            &Request {
                scope: &self.scope,
                consent: self.consent.as_ref(),
            },
            hooks,
        )?;
        serde_json::to_vec(&output)
            .map_err(|_| EditError::Invalid("translation result serialization"))
    }
    pub fn submit(
        self: &Arc<Self>,
        runner: &mut JobRunner,
        session: &Session,
    ) -> crate::Result<(JobId, CancelToken)> {
        if !session.matches_target(&self.target) {
            return Err(EditError::StaleResult);
        }
        let text = self.window.consent_text()?;
        let source = *blake3::hash(text.as_str().as_bytes()).as_bytes();
        let config = self.provider.config();
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Translation,
            source,
            provider: config.id().as_str(),
            model: config.model().as_str(),
            version: config.settings_fingerprint().as_str(),
            settings: &[],
            transcript: source,
            cues: source,
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let run = self.clone();
        runner.submit_for_session(
            session,
            JobSpec {
                kind: JobKind::Translate,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes {
                    compositor: 4 * 1024 * 1024,
                    result: 1024 * 1024,
                    ..Default::default()
                },
            },
            move |hooks: &JobContext| run.run(hooks),
        )
    }
}

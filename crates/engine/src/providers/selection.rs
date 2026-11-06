//! Explicit provider selection, with fresh consent at each attempted provider.
use ai::{
    Consent, CostEstimate, ExecutionHooks, Operation, ProviderConfig, ProviderSelection, Request,
    Scope,
};

/// Job result provenance is the exact configuration that produced the output.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderOutput<T> {
    pub provider: ProviderConfig,
    pub output: T,
}

/// Called immediately before each provider attempt, including every fallback.
/// Composition presents provider/policy link, exact payload and cost, then returns
/// a freshly approved token. `fallback` distinguishes the renewed approval prompt.
/// Returning no token for remote work refuses before any adapter receives data.
pub trait Approval {
    fn approve(
        &mut self,
        scope: &Scope,
        estimate: &CostEstimate,
        fallback: bool,
    ) -> Result<Option<Consent>, ai::Error>;
}

fn eligible(error: &ai::Error) -> bool {
    matches!(
        error,
        ai::Error::Auth
            | ai::Error::Quota
            | ai::Error::Network
            | ai::Error::Server
            | ai::Error::RateLimit { .. }
            | ai::Error::Schema
            | ai::Error::Incomplete
    )
}
fn execute<T>(
    selection: &ProviderSelection,
    operation: Operation,
    configs: &[&ProviderConfig],
    hooks: &dyn ExecutionHooks,
    mut attempt: impl FnMut(usize, bool) -> Result<Result<T, ai::Error>, ai::Error>,
) -> crate::Result<ProviderOutput<T>> {
    if selection.operation() != operation || !selection.providers().eq(configs.iter().copied()) {
        return Err(crate::EditError::Invalid(
            "provider attempts must match configured order",
        ));
    }
    for (index, config) in configs.iter().enumerate() {
        if hooks.cancelled() {
            return Err(ai::Error::Cancelled.into());
        }
        let result = attempt(index, index > 0)?;
        if hooks.cancelled() {
            return Err(ai::Error::Cancelled.into());
        }
        match result {
            Ok(output) => {
                return Ok(ProviderOutput {
                    provider: (*config).clone(),
                    output,
                });
            }
            Err(error) if eligible(&error) && index + 1 < configs.len() => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(crate::EditError::Invalid("missing primary provider"))
}

pub struct TranscriptionAttempt<'a> {
    pub provider: &'a dyn ai::Transcriber,
    pub scope: &'a Scope,
    pub audio: ai::AudioWindow<'a>,
}
/// One recognition attempt per configured provider, no implicit retries or aligner.
/// Text-only output stays explicitly untimed, as in the AI recognition contract.
pub fn transcribe(
    selection: &ProviderSelection,
    attempts: &[TranscriptionAttempt<'_>],
    approval: &mut dyn Approval,
    hooks: &dyn ExecutionHooks,
) -> crate::Result<ProviderOutput<ai::Recognition>> {
    if attempts.len() > 8 {
        return Err(crate::EditError::Limit);
    }
    let configs: Vec<_> = attempts.iter().map(|a| a.provider.config()).collect();
    execute(
        selection,
        Operation::Transcribe,
        &configs,
        hooks,
        |i, fallback| {
            let a = attempts
                .get(i)
                .ok_or(ai::Error::Invalid("missing provider attempt"))?;
            let consent = approval.approve(a.scope, &a.provider.estimate(a.scope), fallback)?;
            Ok(ai::transcribe(
                a.provider,
                ai::AudioWindow {
                    samples: a.audio.samples,
                    range: a.audio.range,
                    sample_rate: a.audio.sample_rate,
                },
                &Request {
                    scope: a.scope,
                    consent: consent.as_ref(),
                },
                None,
                hooks,
            ))
        },
    )
}

pub struct TranslationAttempt<'a> {
    pub provider: &'a dyn ai::Translator,
    pub scope: &'a Scope,
}
pub fn translate(
    selection: &ProviderSelection,
    attempts: &[TranslationAttempt<'_>],
    window: &ai::TranslationWindow,
    approval: &mut dyn Approval,
    hooks: &dyn ExecutionHooks,
) -> crate::Result<ProviderOutput<ai::Translation>> {
    if attempts.len() > 8 {
        return Err(crate::EditError::Limit);
    }
    let configs: Vec<_> = attempts.iter().map(|a| a.provider.config()).collect();
    execute(
        selection,
        Operation::Translate,
        &configs,
        hooks,
        |i, fallback| {
            let a = attempts
                .get(i)
                .ok_or(ai::Error::Invalid("missing provider attempt"))?;
            let consent = approval.approve(a.scope, &a.provider.estimate(a.scope), fallback)?;
            Ok(ai::translate(
                a.provider,
                window,
                &Request {
                    scope: a.scope,
                    consent: consent.as_ref(),
                },
                hooks,
            ))
        },
    )
}

pub struct ClipAttempt<'a> {
    pub provider: &'a dyn ai::clips::ClipSuggester,
    /// One exact scope per input window, in windows() order.
    pub scopes: &'a [Scope],
}
/// A failed provider's partial windows are discarded. All windows of the next
/// provider require new approval before its first upload; heuristics use this same path.
pub fn suggest_clips(
    selection: &ProviderSelection,
    attempts: &[ClipAttempt<'_>],
    input: &ai::clips::ClipInput,
    approval: &mut dyn Approval,
    hooks: &dyn ExecutionHooks,
) -> crate::Result<ProviderOutput<ai::clips::ClipSuggestions>> {
    if attempts.len() > 8 {
        return Err(crate::EditError::Limit);
    }
    let expected_windows = input.windows()?.len();
    if attempts.iter().any(|a| a.scopes.len() != expected_windows) {
        return Err(ai::Error::Invalid("clip window requests").into());
    }
    let configs: Vec<_> = attempts.iter().map(|a| a.provider.config()).collect();
    execute(
        selection,
        Operation::SuggestClips,
        &configs,
        hooks,
        |i, fallback| {
            let a = attempts
                .get(i)
                .ok_or(ai::Error::Invalid("missing provider attempt"))?;
            let consents = a
                .scopes
                .iter()
                .map(|scope| approval.approve(scope, &a.provider.estimate(scope), fallback))
                .collect::<Result<Vec<_>, _>>()?;
            let requests: Vec<_> = a
                .scopes
                .iter()
                .zip(&consents)
                .map(|(scope, consent)| Request {
                    scope,
                    consent: consent.as_ref(),
                })
                .collect();
            Ok(ai::clips::suggest_clips(
                a.provider, input, &requests, hooks,
            ))
        },
    )
}

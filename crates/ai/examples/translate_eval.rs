//! Opt-in, single-attempt evaluation of caller-supplied public text windows.
use ai::{
    Consent, CostEstimate, Money, Operation, Request, SampleRange, Scope, Translation,
    TranslationWindow, Translator,
    openai::{OpenAiTranslator, TRANSLATION_MODE, TRANSLATION_MODEL},
    transport::{ApiKey, UreqTransport},
};
use std::{io::Read, path::PathBuf, sync::Arc};
struct Hooks;
impl ai::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
fn execute() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("VIBECUT_LIVE_AI").as_deref() != Ok("1") {
        return Err("set VIBECUT_LIVE_AI=1 after reviewing disclosure and budget".into());
    }
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().ok_or("expected WINDOWS_JSON OUTPUT_JSON")?);
    let output = PathBuf::from(args.next().ok_or("expected WINDOWS_JSON OUTPUT_JSON")?);
    if args.next().is_some() {
        return Err("expected WINDOWS_JSON OUTPUT_JSON".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&input)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err("evaluation input too large".into());
    }
    if output.exists() && output.canonicalize()? == input.canonicalize()? {
        return Err("evaluation output must differ from input".into());
    }
    let windows: Vec<TranslationWindow> = serde_json::from_slice(&bytes)?;
    if windows.is_empty() || windows.len() > 8 {
        return Err("evaluation admits 1..=8 windows".into());
    }
    let budget: u64 = std::env::var("VIBECUT_TRANSLATE_EVAL_BUDGET_MICROS")?.parse()?;
    let config = OpenAiTranslator::provider_config(TRANSLATION_MODEL)?;
    let provider = OpenAiTranslator::new(
        config.clone(),
        ApiKey::new(std::env::var("OPENAI_API_KEY").map_err(|_| "missing OPENAI_API_KEY")?)?,
        Arc::new(UreqTransport::new()),
    )?;
    let scopes = windows
        .iter()
        .map(|w| {
            Ok(Scope {
                provider: config.clone(),
                operation: Operation::Translate,
                payload_fingerprint: "public-authored-eval-v1".into(),
                mode: TRANSLATION_MODE.into(),
                texts: vec![w.consent_text()?],
                source_language: w.source_language.clone(),
                target_language: Some(w.target_language.clone()),
                source_samples: SampleRange::new(0, 1)?,
                sample_rate: 1,
                max_bytes: 65536,
            })
        })
        .collect::<Result<Vec<_>, ai::Error>>()?;
    let mut ceiling = 0u64;
    for s in &scopes {
        let CostEstimate::UpperBound(cost) = provider.estimate(s) else {
            return Err(ai::Error::Cost.into());
        };
        ceiling = ceiling.checked_add(cost.micros()).ok_or(ai::Error::Cost)?;
        provider.disclosure(s)?;
    }
    if ceiling > budget {
        return Err(ai::Error::Cost.into());
    }
    let mut results: Vec<Translation> = Vec::new();
    for (w, s) in windows.iter().zip(&scopes) {
        let consent = Consent::grant(s.clone(), Money::new("USD", budget)?)?;
        results.push(ai::translate(
            &provider,
            w,
            &Request {
                scope: s,
                consent: Some(&consent),
            },
            &Hooks,
        )?);
        // Persist every completed window for review; a later failure cannot erase it.
        // This is an explicitly requested evaluation file, never project storage.
        std::fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
    }
    Ok(())
}
fn main() {
    if execute().is_err() {
        eprintln!(
            "translation evaluation failed; check input, explicit consent/budget, credentials and provider availability"
        );
        std::process::exit(1);
    }
}

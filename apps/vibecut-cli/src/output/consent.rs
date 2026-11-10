//! Presents explicit upload consent scope.
use engine::EditError;
use engine::providers::{Approval, CatalogOperation, ConsentPrompt, Cost, UploadRequest};
use serde_json::{Value, json};
use std::io;
use std::io::Write as _;

pub(crate) struct CliConsent {
    yes_upload: bool,
}
impl CliConsent {
    pub(crate) fn new(yes_upload: bool) -> Self {
        Self { yes_upload }
    }
}
impl ConsentPrompt for CliConsent {
    fn approve(&self, request: &UploadRequest) -> Result<(), EditError> {
        let operation = match request.operation {
            CatalogOperation::Transcribe => "transcribe",
            CatalogOperation::Translate => "translate",
            CatalogOperation::SuggestClips => "suggest_clips",
            CatalogOperation::GenerateChapters => "generate_chapters",
            CatalogOperation::ClassifyFillers => "classify_fillers",
        };
        let _ = writeln!(
            io::stderr(),
            "upload consent: {operation} {} {} ({})",
            request.provider,
            request.model,
            request.languages
        );
        if request.audio_seconds > 0.0 {
            let _ = writeln!(io::stderr(), "audio: {:.3} s", request.audio_seconds);
        }
        let _ = writeln!(io::stderr(), "payload: {} bytes", request.upload_bytes);
        let _ = writeln!(io::stderr(), "cost: {}", cost_label(&request.cost));
        for (label, value) in &request.terms {
            let _ = writeln!(io::stderr(), "{label}: {value}");
        }
        if self.yes_upload {
            Ok(())
        } else {
            let _ = writeln!(
                io::stderr(),
                "consent: refused; pass --yes-upload to authorize this exact scope"
            );
            Err(engine::providers::Error::ConsentRequired.into())
        }
    }
}

/// The CLI approval for providers without a per-run dialog. `--yes-upload`
/// approves the exact scope printed by [`CliConsent`]; otherwise it refuses.
pub(crate) struct CliApproval {
    prompt: CliConsent,
}
impl CliApproval {
    pub(crate) fn new(yes_upload: bool) -> Self {
        Self {
            prompt: CliConsent::new(yes_upload),
        }
    }
}
impl Approval for CliApproval {
    fn approve(
        &mut self,
        scope: &ai::Scope,
        estimate: &ai::CostEstimate,
        _: bool,
    ) -> Result<Option<ai::Consent>, ai::Error> {
        let endpoint = match scope.provider.location() {
            ai::Location::Remote { base_url, .. } => base_url.as_str().to_owned(),
            ai::Location::Local { model_ref, .. } => model_ref.as_str().to_owned(),
        };
        let upload_bytes = u64::try_from(scope.texts.first().map_or(0, |text| text.as_str().len()))
            .map_err(|_| ai::Error::Payload)?;
        let request = UploadRequest {
            operation: CatalogOperation::GenerateChapters,
            provider: scope.provider.id().as_str().into(),
            model: scope.provider.model().as_str().into(),
            languages: scope.source_language.as_str().into(),
            audio_seconds: 0.0,
            upload_bytes,
            cost: Cost::from(estimate),
            terms: vec![("endpoint", endpoint)],
        };
        self.prompt
            .approve(&request)
            .map_err(|_| ai::Error::ConsentRequired)?;
        let budget = match estimate {
            ai::CostEstimate::Free => ai::Money::new("USD", 0)?,
            ai::CostEstimate::UpperBound(money) => money.clone(),
            ai::CostEstimate::Unknown => return Err(ai::Error::Cost),
        };
        Ok(Some(ai::Consent::grant(scope.clone(), budget)?))
    }
}

pub(crate) fn cost_label(cost: &Cost) -> String {
    match cost {
        Cost::Free => "free".into(),
        Cost::Unknown => "unknown".into(),
        Cost::UpperBound { currency, micros } => {
            format!("{currency} {:.6}", *micros as f64 / 1_000_000.0)
        }
    }
}
pub(crate) fn cost_json(cost: &Cost) -> Value {
    match cost {
        Cost::Free => json!({"kind": "free"}),
        Cost::Unknown => json!({"kind": "unknown"}),
        Cost::UpperBound { currency, micros } => {
            json!({"kind": "upper_bound", "currency": currency, "micros": micros})
        }
    }
}

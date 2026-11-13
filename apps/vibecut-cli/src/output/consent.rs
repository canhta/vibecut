//! Presents explicit upload consent scope.
use engine::providers::{CatalogOperation, ConsentPrompt, Cost, UploadRequest};
use serde_json::{Value, json};
use std::io;
use std::io::Write as _;

pub(crate) struct CliConsent {
    yes_upload: bool,
    accept_unknown_price: bool,
}
impl CliConsent {
    pub(crate) fn new(yes_upload: bool, accept_unknown_price: bool) -> Self {
        Self {
            yes_upload,
            accept_unknown_price,
        }
    }
}
impl ConsentPrompt for CliConsent {
    fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        if !self.accept_unknown_price {
            return Err(ai::Error::Cost.into());
        }
        self.approve(request)
    }
    fn approve(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        let operation = match request.operation {
            CatalogOperation::Transcribe => "transcribe",
            CatalogOperation::Translate => "translate",
            CatalogOperation::SuggestClips => "suggest_clips",
            CatalogOperation::GenerateChapters => "generate_chapters",
            CatalogOperation::ClassifyFillers => "classify_fillers",
            CatalogOperation::SelectEmphasis => "select_emphasis",
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
            Err(engine::providers::Error::ConsentRequired)
        }
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
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use engine::providers::Error as ProviderError;
    #[test]
    fn cli_model_selection_and_unknown_price_flag_are_independent_from_upload_approval() {
        let selected =
            engine::providers::choose_chapter("openai", Some("fixture-cli-model")).unwrap();
        let engine::providers::TextProvider::OpenAi { model } = selected else {
            panic!("provider")
        };
        assert_eq!(model, "fixture-cli-model");
        let request = UploadRequest {
            config: ai::text::chapters::OpenAiChapterGenerator::provider_config(&model).unwrap(),
            operation: CatalogOperation::GenerateChapters,
            provider: "OpenAI".into(),
            model,
            languages: "en".into(),
            audio_seconds: 0.0,
            upload_bytes: 10,
            cost: Cost::Unknown,
            terms: vec![],
        };
        assert!(matches!(
            CliConsent::new(true, false).approve_unknown_price(&request),
            Err(ProviderError::Ai(ai::Error::Cost))
        ));
        assert!(
            CliConsent::new(false, true)
                .approve_unknown_price(&request)
                .is_err()
        );
        CliConsent::new(true, true)
            .approve_unknown_price(&request)
            .unwrap();
    }
}

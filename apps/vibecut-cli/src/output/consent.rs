//! Presents explicit upload consent scope.
use engine::EditError;
use engine::providers::{CatalogOperation, ConsentPrompt, Cost, UploadRequest};
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
            Err(EditError::ConsentRequired)
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

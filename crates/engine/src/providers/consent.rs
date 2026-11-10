//! Discloses each remote upload before it is approved.
use super::catalog::CatalogOperation;
use super::error::Error;
use crate::EditError;
use ai::{CostEstimate, Money};
/// A provider disclosure shown before any remote byte leaves the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cost {
    Free,
    UpperBound { currency: String, micros: u64 },
    Unknown,
}
#[derive(Clone, Debug)]
pub struct UploadRequest {
    pub operation: CatalogOperation,
    pub provider: String,
    pub model: String,
    pub languages: String,
    pub audio_seconds: f64,
    pub upload_bytes: u64,
    pub cost: Cost,
    pub terms: Vec<(&'static str, String)>,
}
/// The adapter refuses the remote request when this returns `Err(ConsentRequired)`.
pub trait ConsentPrompt {
    fn approve(&self, request: &UploadRequest) -> Result<(), EditError>;
}
/// A prompt that refuses every remote request. Test fixtures and non-interactive
/// callers must opt in explicitly with [`AllowAll`].
pub struct DenyUploads;
pub struct AllowAll;
impl Cost {
    /// Accumulate an estimate, saturating micros and marking mixed currencies unknown.
    pub fn add(&mut self, next: &CostEstimate) {
        match (&mut *self, next) {
            (Self::Unknown, _) => {}
            (_, CostEstimate::Unknown) => *self = Self::Unknown,
            (Self::Free, CostEstimate::Free) => {}
            (Self::Free, CostEstimate::UpperBound(money)) => {
                *self = Self::UpperBound {
                    currency: money.currency().into(),
                    micros: money.micros(),
                };
            }
            (Self::UpperBound { currency, micros }, CostEstimate::Free) => {
                let _ = (currency, micros);
            }
            (Self::UpperBound { currency, micros }, CostEstimate::UpperBound(money))
                if currency == money.currency() =>
            {
                *micros = micros.saturating_add(money.micros());
            }
            (Self::UpperBound { .. }, CostEstimate::UpperBound(_)) => *self = Self::Unknown,
        }
    }
}
impl From<&CostEstimate> for Cost {
    fn from(value: &CostEstimate) -> Self {
        match value {
            CostEstimate::Free => Self::Free,
            CostEstimate::UpperBound(money) => Self::UpperBound {
                currency: money.currency().into(),
                micros: money.micros(),
            },
            CostEstimate::Unknown => Self::Unknown,
        }
    }
}
impl ConsentPrompt for DenyUploads {
    fn approve(&self, _: &UploadRequest) -> Result<(), EditError> {
        Err(Error::ConsentRequired.into())
    }
}
impl ConsentPrompt for AllowAll {
    fn approve(&self, _: &UploadRequest) -> Result<(), EditError> {
        Ok(())
    }
}
pub(crate) fn estimate_money(estimate: &CostEstimate) -> Result<Money, EditError> {
    match estimate {
        CostEstimate::Free => Ok(Money::new("USD", 0)?),
        CostEstimate::UpperBound(money) => Ok(money.clone()),
        CostEstimate::Unknown => Err(EditError::Ai(ai::Error::Cost)),
    }
}

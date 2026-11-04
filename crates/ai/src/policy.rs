use crate::{Error, Operation, SampleRange};
use captions::{LanguageTag, NfcText};
use serde::{Deserialize, Serialize};

/// Tagged, versioned provider settings; secrets are resolved externally from auth_ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ConfigData", into = "ConfigData")]
pub struct ProviderConfig(ConfigData);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigData {
    schema_version: u32,
    id: NfcText,
    model: NfcText,
    settings_fingerprint: NfcText,
    location: Location,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    Local {
        backend: NfcText,
        model_ref: NfcText,
    },
    Remote {
        backend: NfcText,
        base_url: NfcText,
        auth_ref: NfcText,
    },
}
impl ProviderConfig {
    pub fn new(
        id: NfcText,
        model: NfcText,
        settings_fingerprint: NfcText,
        location: Location,
    ) -> Result<Self, Error> {
        ConfigData {
            schema_version: 1,
            id,
            model,
            settings_fingerprint,
            location,
        }
        .try_into()
    }
    pub fn id(&self) -> &NfcText {
        &self.0.id
    }
    pub fn model(&self) -> &NfcText {
        &self.0.model
    }
    pub fn settings_fingerprint(&self) -> &NfcText {
        &self.0.settings_fingerprint
    }
    pub fn location(&self) -> &Location {
        &self.0.location
    }
    pub fn is_remote(&self) -> bool {
        matches!(self.location(), Location::Remote { .. })
    }
}
impl TryFrom<ConfigData> for ProviderConfig {
    type Error = Error;
    fn try_from(data: ConfigData) -> Result<Self, Error> {
        if data.schema_version != 1 {
            return Err(Error::Invalid("provider schema version"));
        }
        for value in [&data.id, &data.model, &data.settings_fingerprint] {
            nonempty(value)?;
        }
        match &data.location {
            Location::Local { backend, model_ref } => {
                nonempty(backend)?;
                nonempty(model_ref)?;
            }
            Location::Remote {
                backend,
                base_url,
                auth_ref,
            } => {
                nonempty(backend)?;
                nonempty(auth_ref)?;
                let url = url::Url::parse(base_url.as_str())
                    .map_err(|_| Error::Invalid("provider URL"))?;
                if url.scheme() != "https"
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(Error::Invalid(
                        "provider URL must be HTTPS without credentials, query or fragment",
                    ));
                }
            }
        }
        Ok(Self(data))
    }
}
impl From<ProviderConfig> for ConfigData {
    fn from(value: ProviderConfig) -> Self {
        value.0
    }
}
pub(crate) fn nonempty(value: &NfcText) -> Result<(), Error> {
    if value.as_str().trim().is_empty() {
        Err(Error::Invalid("empty identity"))
    } else {
        Ok(())
    }
}

/// Integer millionths of a currency unit, never floating-point billing arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MoneyData", into = "MoneyData")]
pub struct Money(MoneyData);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoneyData {
    currency: String,
    micros: u64,
}
impl Money {
    pub fn new(currency: &str, micros: u64) -> Result<Self, Error> {
        MoneyData {
            currency: currency.into(),
            micros,
        }
        .try_into()
    }
    pub fn micros(&self) -> u64 {
        self.0.micros
    }
    pub fn currency(&self) -> &str {
        &self.0.currency
    }
    fn covers(&self, other: &Self) -> bool {
        self.currency() == other.currency() && self.micros() >= other.micros()
    }
}
impl TryFrom<MoneyData> for Money {
    type Error = Error;
    fn try_from(value: MoneyData) -> Result<Self, Error> {
        if value.currency.len() != 3 || !value.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(Error::Invalid("currency"));
        }
        Ok(Self(value))
    }
}
impl From<Money> for MoneyData {
    fn from(value: Money) -> Self {
        value.0
    }
}
/// Unknown is distinct from free. Remote execution requires a bounded estimate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CostEstimate {
    Free,
    UpperBound(Money),
    Unknown,
}

/// Exact request identity. Fingerprints bind the source audio and, for align/translate,
/// audio content; text payloads are bound exactly in memory and redact Debug.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub provider: ProviderConfig,
    pub operation: Operation,
    pub payload_fingerprint: NfcText,
    pub mode: NfcText,
    pub texts: Vec<NfcText>,
    pub source_language: LanguageTag,
    pub target_language: Option<LanguageTag>,
    pub source_samples: SampleRange,
    pub sample_rate: u32,
    /// Wire-payload ceiling (encoded upload bytes), not in-memory f32 PCM.
    pub max_bytes: u64,
}
/// In-memory approval only; never deserialized from projects or provider responses.
#[derive(Clone, Debug)]
pub struct Consent {
    scope: Scope,
    budget: Money,
}
impl Consent {
    /// The composition root calls this only after affirmative approval of the scope/budget.
    pub fn grant(scope: Scope, budget: Money) -> Result<Self, Error> {
        scope.validate()?;
        Ok(Self { scope, budget })
    }
}
impl Scope {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        nonempty(&self.payload_fingerprint)?;
        nonempty(&self.mode)?;
        if self.sample_rate == 0 {
            return Err(Error::Invalid("sample rate"));
        }
        if (self.operation == Operation::Translate) != self.target_language.is_some() {
            return Err(Error::Invalid("target language"));
        }
        Ok(())
    }
}
/// Shared policy for every execution seam, including fallback aligners.
pub fn authorize(
    scope: &Scope,
    estimate: &CostEstimate,
    consent: Option<&Consent>,
) -> Result<(), Error> {
    scope.validate()?;
    if !scope.provider.is_remote() {
        return Ok(());
    }
    let token = consent.ok_or(Error::ConsentRequired)?;
    let mut allowed = token.scope.clone();
    // A chunk/retry may shrink the approved source window and upload size only.
    if !allowed.source_samples.contains(scope.source_samples) || scope.max_bytes > allowed.max_bytes
    {
        return Err(Error::ConsentRequired);
    }
    allowed.source_samples = scope.source_samples;
    allowed.max_bytes = scope.max_bytes;
    if allowed != *scope {
        return Err(Error::ConsentRequired);
    }
    match estimate {
        CostEstimate::Free => Ok(()),
        CostEstimate::UpperBound(cost) if token.budget.covers(cost) => Ok(()),
        _ => Err(Error::Cost),
    }
}

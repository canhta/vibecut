//! Shipped remote model choices with their documented price data.
use crate::{CostEstimate, Error, Money, Operation, ProviderConfig};

pub const MAX_MODEL_BYTES: usize = 256;

/// Conservative quoting rates in micro-USD. Token prices are per million tokens.
/// Known rates retain existing admission ceilings; they are not measured billing.
#[derive(Clone, Copy, Debug)]
pub enum Price {
    AudioHour(u64),
    Text {
        input: u64,
        output: u64,
    },
    Speech {
        text_input: u64,
        audio_input: u64,
        text_output: u64,
        audio_output: u64,
    },
    Character {
        micros: u64,
        max_characters: u64,
        language_code: bool,
        vietnamese: bool,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct ModelDefault {
    pub provider: &'static str,
    pub feature: Operation,
    pub model: &'static str,
    pub price: Price,
    pub shipped_default: bool,
}

// Provider docs and pricing checked 2026-11-11; links live in contracts/README.md.
// Values are data: adapters never use model identity as an admission allowlist.
pub const DEFAULTS: &[ModelDefault] = &[
    ModelDefault {
        provider: "openai",
        feature: Operation::Transcribe,
        model: "gpt-transcribe",
        price: Price::AudioHour(270_000),
        shipped_default: true,
    },
    ModelDefault {
        provider: "elevenlabs",
        feature: Operation::Transcribe,
        model: "scribe_v2",
        price: Price::AudioHour(220_000),
        shipped_default: true,
    },
    ModelDefault {
        provider: "openai",
        feature: Operation::Translate,
        model: "gpt-6.1-sol",
        price: Price::Text {
            input: 2_000_000,
            output: 10_000_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "anthropic",
        feature: Operation::Translate,
        model: "claude-sonnet-5-5",
        price: Price::Text {
            input: 2_000_000,
            output: 10_000_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "openai",
        feature: Operation::SuggestClips,
        model: "gpt-6-luna",
        price: Price::Text {
            input: 100_000,
            output: 500_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "anthropic",
        feature: Operation::SuggestClips,
        model: "claude-sonnet-5-5",
        price: Price::Text {
            input: 2_000_000,
            output: 10_000_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "openai",
        feature: Operation::SuggestClips,
        model: "gpt-6.1-sol",
        price: Price::Text {
            input: 2_000_000,
            output: 10_000_000,
        },
        shipped_default: false,
    },
    ModelDefault {
        provider: "openai",
        feature: Operation::GenerateChapters,
        model: "gpt-6-luna",
        price: Price::Text {
            input: 100_000,
            output: 500_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "anthropic",
        feature: Operation::GenerateChapters,
        model: "claude-sonnet-5-5",
        price: Price::Text {
            input: 2_000_000,
            output: 10_000_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "openai",
        feature: Operation::ClassifyFillers,
        model: "gpt-6-luna",
        price: Price::Text {
            input: 100_000,
            output: 500_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "anthropic",
        feature: Operation::ClassifyFillers,
        model: "claude-sonnet-5-5",
        price: Price::Text {
            input: 2_000_000,
            output: 10_000_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "openai",
        feature: Operation::Synthesize,
        model: "gpt-audio-1.5",
        price: Price::Speech {
            text_input: 2_500_000,
            audio_input: 32_000_000,
            text_output: 10_000_000,
            audio_output: 64_000_000,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "elevenlabs",
        feature: Operation::Synthesize,
        model: "eleven_flash_v2_5",
        price: Price::Character {
            micros: 40,
            max_characters: 40_000,
            language_code: true,
            vietnamese: true,
        },
        shipped_default: true,
    },
    ModelDefault {
        provider: "elevenlabs",
        feature: Operation::Synthesize,
        model: "eleven_multilingual_v2",
        price: Price::Character {
            micros: 80,
            max_characters: 10_000,
            language_code: false,
            vietnamese: false,
        },
        shipped_default: false,
    },
];

/// An unsupported provider/feature pair has no shipped default.
pub fn model(provider: &str, feature: Operation) -> &'static str {
    DEFAULTS
        .iter()
        .find(|row| row.provider == provider && row.feature == feature && row.shipped_default)
        .map_or("", |row| row.model)
}
pub fn validate_model(model: &str) -> Result<(), Error> {
    if model.trim().is_empty()
        || model.len() > MAX_MODEL_BYTES
        || model.chars().any(char::is_control)
    {
        return Err(Error::Invalid("remote model id"));
    }
    Ok(())
}
pub fn price(config: &ProviderConfig, feature: Operation) -> Option<Price> {
    DEFAULTS
        .iter()
        .find(|row| {
            row.provider == config.id().as_str()
                && row.feature == feature
                && row.model == config.model().as_str()
        })
        .map(|row| row.price)
}
pub(crate) fn text_estimate(
    config: &ProviderConfig,
    feature: Operation,
    input_tokens: u64,
    output_tokens: u64,
) -> CostEstimate {
    let Some(Price::Text { input, output }) = price(config, feature) else {
        return CostEstimate::Unknown;
    };
    let input = input.max(2_000_000);
    let output = output.max(10_000_000);
    let micros = (u128::from(input_tokens) * u128::from(input)
        + u128::from(output_tokens) * u128::from(output))
    .div_ceil(1_000_000);
    u64::try_from(micros)
        .ok()
        .and_then(|micros| Money::new("USD", micros).ok())
        .map_or(CostEstimate::Unknown, CostEstimate::UpperBound)
}
pub(crate) fn audio_estimate(config: &ProviderConfig, seconds: u128) -> CostEstimate {
    let Some(Price::AudioHour(rate)) = price(config, Operation::Transcribe) else {
        return CostEstimate::Unknown;
    };
    seconds
        .checked_mul(u128::from(rate))
        .map(|n| n.div_ceil(3600))
        .and_then(|n| u64::try_from(n).ok())
        .and_then(|n| Money::new("USD", n).ok())
        .map_or(CostEstimate::Unknown, CostEstimate::UpperBound)
}

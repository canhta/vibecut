//! Per-conversation spend ledger and the token price table behind it.
use super::Usage;
use crate::{Error, ProviderConfig};

/// One in-flight reservation at a time: a request reserves its upper bound
/// before it is sent and settles to what it cost after, so the limit can never
/// be passed by the request that is running.
#[derive(Clone, Debug)]
pub struct Ledger {
    limit: u64,
    settled: u64,
    reserved: u64,
}
impl Ledger {
    pub fn new(limit_micros: u64) -> Self {
        Self {
            limit: limit_micros,
            settled: 0,
            reserved: 0,
        }
    }
    /// Refuses, changing nothing, when the settled spend plus this bound passes the limit.
    pub fn reserve(&mut self, upper_micros: u64) -> Result<(), Error> {
        if self.reserved != 0 {
            return Err(Error::Invalid("a request is already reserved"));
        }
        match self.settled.checked_add(upper_micros) {
            Some(total) if total <= self.limit => {
                self.reserved = upper_micros;
                Ok(())
            }
            _ => Err(Error::Cost),
        }
    }
    /// Replace the reservation with what the request cost (zero when it never
    /// reached generation).
    pub fn settle(&mut self, actual_micros: u64) {
        self.settled = self.settled.saturating_add(actual_micros);
        self.reserved = 0;
    }
    pub fn spent_micros(&self) -> u64 {
        self.settled
    }
    pub fn limit_micros(&self) -> u64 {
        self.limit
    }
}

/// Micro-USD per one million tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Price {
    input: u64,
    output: u64,
}
// Docs checked 2026-11-12: Claude Sonnet 5.5 ($2/$10, platform.claude.com/docs/en/about-claude/pricing),
// GPT-6.1 Sol ($2/$10) and GPT-6 Astra ($10/$50), both for prompts up to 272K input
// tokens (developers.openai.com/api/docs/models). Longer prompts are priced higher, which
// `MAX_CONTEXT_BYTES` keeps out of reach: a token is at least one byte.
const TABLE: &[(&str, &str, Price)] = &[
    (
        "anthropic",
        "claude-sonnet-5-5",
        Price {
            input: 2_000_000,
            output: 10_000_000,
        },
    ),
    (
        "openai",
        "gpt-6.1-sol",
        Price {
            input: 2_000_000,
            output: 10_000_000,
        },
    ),
    (
        "openai",
        "gpt-6-astra",
        Price {
            input: 10_000_000,
            output: 50_000_000,
        },
    ),
];
/// The model a provider's agent chat starts with, as of the docs above.
pub fn default_model(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("claude-sonnet-5-5"),
        "openai" => Some("gpt-6.1-sol"),
        _ => None,
    }
}
/// A model without a registered price cannot be held to a spend limit.
pub(super) fn price(config: &ProviderConfig) -> Option<Price> {
    TABLE
        .iter()
        .find(|(provider, model, _)| {
            *provider == config.id().as_str() && *model == config.model().as_str()
        })
        .map(|(_, _, price)| *price)
}
impl Price {
    /// A token is at least one byte, so the body length bounds the input tokens.
    pub(super) fn upper_bound(self, body_bytes: u64, max_output_tokens: u64) -> Result<u64, Error> {
        self.cost(body_bytes, max_output_tokens)
    }
    pub(super) fn usage(self, usage: Usage) -> Result<u64, Error> {
        self.cost(usage.input_tokens, usage.output_tokens)
    }
    fn cost(self, input_tokens: u64, output_tokens: u64) -> Result<u64, Error> {
        let micros = (u128::from(input_tokens) * u128::from(self.input)
            + u128::from(output_tokens) * u128::from(self.output))
        .div_ceil(1_000_000);
        u64::try_from(micros).map_err(|_| Error::Cost)
    }
}

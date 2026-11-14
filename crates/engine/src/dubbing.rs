//! Synthesizes narration takes with the provider the user chose and fits them to cues.
mod narrator;
mod speech;
mod take;

pub use crate::session::{
    DurationFit, Fit, FitInput, OVERFLOW_DENOMINATOR, OVERFLOW_NUMERATOR,
    PROVIDER_SPEED_MAX_THOUSANDTHS, STRETCH_MAX_THOUSANDTHS, STRETCH_MIN_THOUSANDTHS, fit,
};
pub use narrator::{
    CancelFlag, Disclosure, MAX_TAKE_BYTES, Narrator, Prepared, Request, SpeechOpener,
};
pub use speech::{CredentialSpeech, ELEVENLABS_VOICES, SpeechOption, speech_options};
pub use take::{DubTake, decode_take, encode_take, ticks_of, wav_bytes, write_wav};

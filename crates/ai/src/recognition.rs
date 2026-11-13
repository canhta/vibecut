//! Transcription and alignment engines: local Whisper and Gipformer, Candle alignment and recognition assembly.
#[cfg(feature = "candle-align")]
pub mod alignment;
pub(crate) mod assembly;
#[cfg(feature = "gipformer")]
pub mod gipformer;
#[cfg(feature = "whisper")]
pub mod whisper;

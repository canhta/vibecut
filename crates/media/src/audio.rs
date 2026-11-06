//! Extracts speech audio, resamples continuous audio spans and prepares upload chunks.

mod chunk;
mod energy;
mod extract;
mod resample;

pub use chunk::{
    AudioChunk, SpeechExtractSettings, SpeechWindow, extract_speech_audio,
    extract_speech_audio_with, extract_speech_audio_with_cancel,
    extract_speech_windows_with_cancel,
};
pub use energy::{SpeechEnergyWindow, analyze_speech_energy};
pub use extract::SPEECH_SAMPLE_RATE;
pub use resample::{AudioResampler, AudioSampleFormat};

//! Streams bounded speech windows, measures speech energy, summarizes source
//! peaks and resamples continuous audio spans.

mod chunk;
mod energy;
mod extract;
mod peaks;
mod resample;

pub use chunk::{SpeechExtractSettings, SpeechWindow, extract_speech_windows_with_cancel};
pub use energy::{SpeechEnergyWindow, analyze_speech_energy};
pub use extract::SPEECH_SAMPLE_RATE;
pub use peaks::{
    BucketPeak, PEAK_BUCKET_SAMPLES, PEAK_SUMMARY_MAX_BYTES, PeakSummary, peak_summary,
};
pub use resample::{AudioResampler, AudioSampleFormat};

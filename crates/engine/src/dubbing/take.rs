//! A generated take: mono PCM16, its provenance and its file form.
use ai::Error;
use project::Narration;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use time::Tick;

/// One synthesized take as the job returns it. The audio is the provider's mono PCM16,
/// untouched: fitting happens on the timeline through the retime path, never here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DubTake {
    pub sample_rate: u32,
    pub pcm: Vec<i16>,
    /// What produced it and what the user approved; becomes the item's provenance.
    pub narration: Narration,
}
impl DubTake {
    /// The natural duration of the audio at speed 1.
    pub fn natural(&self) -> Result<Tick, Error> {
        ticks_of(self.pcm.len() as u64, self.sample_rate)
    }
    /// A short stable file stem: the text hash and the sample count.
    pub fn stem(&self) -> String {
        let hash = self.narration.text_sha256.as_str();
        format!(
            "narration-{}-{}",
            hash.get(..16).unwrap_or(hash),
            self.pcm.len()
        )
    }
}

/// Ticks of `samples` at `rate`, rounded up so the item never ends before the audio.
pub fn ticks_of(samples: u64, rate: u32) -> Result<Tick, Error> {
    if rate == 0 {
        return Err(Error::Invalid("sample rate"));
    }
    let ticks = (i128::from(samples) * i128::from(time::TICKS_PER_SECOND) + i128::from(rate) - 1)
        / i128::from(rate);
    i64::try_from(ticks)
        .map(Tick)
        .map_err(|_| Error::Invalid("take duration"))
}

/// Serialize a take as job output.
pub fn encode_take(take: &DubTake) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(take).map_err(|_| Error::Invalid("narration take"))
}
/// Parse job output. Unknown fields and malformed provenance refuse.
pub fn decode_take(bytes: &[u8]) -> Result<DubTake, Error> {
    let take: DubTake =
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid("narration take"))?;
    take.narration
        .validate()
        .map_err(|_| Error::Invalid("narration take"))?;
    if take.pcm.is_empty() || take.sample_rate == 0 {
        return Err(Error::Invalid("narration take"));
    }
    Ok(take)
}

/// A canonical 44-byte-header mono PCM16 WAV.
pub fn wav_bytes(sample_rate: u32, pcm: &[i16]) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Write the take's WAV into `directory` under its stem and return the path. An existing
/// file with the same stem is the same take and is reused; the write is staged and renamed.
pub fn write_wav(directory: &Path, take: &DubTake) -> std::io::Result<PathBuf> {
    let path = directory.join(format!("{}.wav", take.stem()));
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(directory)?;
    let staged = directory.join(format!("{}.wav.part", take.stem()));
    std::fs::write(&staged, wav_bytes(take.sample_rate, &take.pcm))?;
    std::fs::rename(&staged, &path)?;
    Ok(path)
}

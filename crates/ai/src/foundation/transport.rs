//! Shared provider credentials and payload encoding: redacted API keys, PCM16 WAV
//! and multipart form construction. Endpoint policy stays in each adapter.
mod text_model;
use crate::{AudioWindow, Error};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
pub(crate) use text_model::{MAX_OUTPUT, TextModel, TextRequest};

/// In-memory API key injected by the composition root. Never serialized or logged.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);
impl ApiKey {
    /// Validates a printable ASCII key of at most 4096 bytes; Debug always redacts it.
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || !value.is_ascii() || value.len() > 4096 {
            return Err(Error::Invalid("API key"));
        }
        Ok(Self(value))
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey([redacted])")
    }
}

/// Encoded PCM16 WAV size including the fixed 44-byte header.
pub(crate) fn wav_bytes(samples: u64) -> Result<u64, Error> {
    samples
        .checked_mul(2)
        .and_then(|data| data.checked_add(44))
        .ok_or(Error::Payload)
}

/// Mono 16-bit PCM WAV. Input is already validated finite mono f32.
pub(crate) fn wav(audio: &AudioWindow<'_>) -> Result<Vec<u8>, Error> {
    let data_bytes = audio.samples.len().checked_mul(2).ok_or(Error::Payload)?;
    let total = data_bytes.checked_add(44).ok_or(Error::Payload)?;
    let data_len = u32::try_from(data_bytes).map_err(|_| Error::Payload)?;
    let riff_len = data_len.checked_add(36).ok_or(Error::Payload)?;
    let byte_rate = audio.sample_rate.checked_mul(2).ok_or(Error::Payload)?;
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"RIFF");
    output.extend_from_slice(&riff_len.to_le_bytes());
    output.extend_from_slice(b"WAVEfmt ");
    output.extend_from_slice(&16u32.to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&audio.sample_rate.to_le_bytes());
    output.extend_from_slice(&byte_rate.to_le_bytes());
    output.extend_from_slice(&2u16.to_le_bytes());
    output.extend_from_slice(&16u16.to_le_bytes());
    output.extend_from_slice(b"data");
    output.extend_from_slice(&data_len.to_le_bytes());
    for sample in audio.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        output.extend_from_slice(&value.to_le_bytes());
    }
    Ok(output)
}

/// Bounded multipart builder shared by provider adapters. Values are trusted
/// adapter metadata; the boundary is chosen so it cannot occur in the audio.
pub(crate) struct Multipart {
    boundary: String,
    body: Vec<u8>,
}
impl Multipart {
    pub(crate) fn new(audio: &[u8]) -> Result<Self, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..8 {
            let boundary = format!(
                "vibecut-{:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            if !audio
                .windows(boundary.len())
                .any(|window| window == boundary.as_bytes())
            {
                return Ok(Self {
                    boundary,
                    body: Vec::new(),
                });
            }
        }
        Err(Error::Invalid("multipart boundary"))
    }
    pub(crate) fn field(&mut self, name: &str, value: &str) {
        self.body
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"").as_bytes(),
        );
        self.body.extend_from_slice(b"\r\n\r\n");
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
    }
    pub(crate) fn file(&mut self, filename: &str, content_type: &str, bytes: &[u8]) {
        self.body
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"")
                .as_bytes(),
        );
        self.body
            .extend_from_slice(format!("\r\nContent-Type: {content_type}\r\n\r\n").as_bytes());
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
    }
    pub(crate) fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

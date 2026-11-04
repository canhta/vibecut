//! Label files: one JSON document per source video, plus synthetic fixture sources.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use time::{FrameRate, Rounding, TICKS_PER_SECOND, Tick};

/// One human-picked short: ordered source segments, gaps allowed.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelledClip {
    pub id: String,
    pub priority: Priority,
    pub segments: Vec<LabelSegment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Must,
    Nice,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelSegment {
    pub start: Tick,
    pub end: Tick,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticSource {
    pub duration_seconds: f64,
}

/// Pipeline settings shared by every video in one label set unless overridden.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    pub count: usize,
    pub min_duration_seconds: f64,
    pub max_duration_seconds: f64,
    pub min_segment_duration_seconds: f64,
    pub max_segments: usize,
    pub overlap_tolerance: f64,
    pub frame_rate: f64,
    /// Maximum absolute source-edge movement during frame snapping.
    pub snap_tolerance_seconds: f64,
    pub silence_rms: f64,
    pub keywords: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            count: 5,
            min_duration_seconds: 20.0,
            max_duration_seconds: 60.0,
            min_segment_duration_seconds: 2.0,
            max_segments: 4,
            overlap_tolerance: 0.5,
            frame_rate: 30.0,
            snap_tolerance_seconds: 1.0,
            silence_rms: 0.01,
            keywords: Vec::new(),
        }
    }
}

/// Validated settings in ticks and rates.
#[derive(Clone, Debug)]
pub struct ResolvedSettings {
    pub count: usize,
    pub min_duration: Tick,
    pub max_duration: Tick,
    pub min_segment_duration: Tick,
    pub max_segments: usize,
    pub overlap_tolerance: f64,
    pub frame_rate: FrameRate,
    pub snap_tolerance: Tick,
    pub silence_rms: f64,
    pub keywords: Vec<String>,
}

impl Settings {
    pub fn resolve(&self) -> Result<ResolvedSettings, String> {
        let seconds = |value: f64, name: &str| -> Result<Tick, String> {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("settings.{name} must be finite and positive"));
            }
            Tick::from_seconds_f64(value, Rounding::Nearest).map_err(|e| e.to_string())
        };
        if !(1..=32).contains(&self.count)
            || !(1..=32).contains(&self.max_segments)
            || !self.overlap_tolerance.is_finite()
            || !(0.0..=1.0).contains(&self.overlap_tolerance)
            || !self.frame_rate.is_finite()
            || !(1.0..=240.0).contains(&self.frame_rate)
            || !self.silence_rms.is_finite()
            || !(0.0..=1.0).contains(&self.silence_rms)
            || self.keywords.len() > 64
        {
            return Err("invalid settings bounds".into());
        }
        let min_duration = seconds(self.min_duration_seconds, "min_duration_seconds")?;
        let max_duration = seconds(self.max_duration_seconds, "max_duration_seconds")?;
        let min_segment_duration = seconds(
            self.min_segment_duration_seconds,
            "min_segment_duration_seconds",
        )?;
        let snap_tolerance = seconds(self.snap_tolerance_seconds, "snap_tolerance_seconds")?;
        if max_duration < min_duration || min_segment_duration > max_duration {
            return Err("inconsistent duration settings".into());
        }
        let scaled = (self.frame_rate * 1000.0).round();
        if !scaled.is_finite() || scaled < 1.0 || scaled > 240_000.0 {
            return Err("settings.frame_rate is out of range".into());
        }
        let frame_rate = FrameRate::new(scaled as i64, 1000).map_err(|e| e.to_string())?;
        Ok(ResolvedSettings {
            count: self.count,
            min_duration,
            max_duration,
            min_segment_duration,
            max_segments: self.max_segments,
            overlap_tolerance: self.overlap_tolerance,
            frame_rate,
            snap_tolerance,
            silence_rms: self.silence_rms,
            keywords: self.keywords.clone(),
        })
    }
}

/// One label file. Paths are relative to the file's directory.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelFile {
    pub version: u32,
    #[serde(default)]
    pub source: Option<PathBuf>,
    pub sha256: String,
    pub language: String,
    #[serde(default)]
    pub transcript: Option<PathBuf>,
    #[serde(default)]
    pub synthetic: Option<SyntheticSource>,
    #[serde(default)]
    pub settings: Settings,
    pub clips: Vec<LabelledClip>,
}

impl LabelFile {
    pub fn load(path: &Path) -> Result<(Self, PathBuf), LabelError> {
        let bytes = std::fs::read(path).map_err(LabelError::Io)?;
        let label: Self = serde_json::from_slice(&bytes).map_err(LabelError::Json)?;
        if label.version != 1 {
            return Err(LabelError::Version(label.version));
        }
        let dir = path
            .parent()
            .ok_or_else(|| LabelError::Invalid("label file has no parent directory".into()))?
            .to_path_buf();
        label.validate()?;
        Ok((label, dir))
    }

    fn validate(&self) -> Result<(), LabelError> {
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(LabelError::MissingHash);
        }
        if self.source.is_none() && self.synthetic.is_none() {
            return Err(LabelError::Invalid(
                "source or synthetic is required".into(),
            ));
        }
        if self.clips.is_empty() {
            return Err(LabelError::Invalid("clips is empty".into()));
        }
        for clip in &self.clips {
            if clip.id.trim().is_empty() || clip.segments.is_empty() {
                return Err(LabelError::Invalid("empty clip id or segments".into()));
            }
            for segment in &clip.segments {
                if segment.end <= segment.start {
                    return Err(LabelError::Invalid(
                        "empty or reversed label segment".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn expected_hash(&self) -> &str {
        &self.sha256
    }

    pub fn source_path(
        &self,
        dir: &Path,
        synthetic_dir: &Path,
        name: &str,
    ) -> Result<PathBuf, LabelError> {
        if let Some(synthetic) = &self.synthetic {
            if !synthetic.duration_seconds.is_finite() || synthetic.duration_seconds <= 0.0 {
                return Err(LabelError::Invalid("synthetic duration".into()));
            }
            return Ok(synthetic_dir.join(format!("{name}.wav")));
        }
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| LabelError::Invalid("source is required".into()))?;
        Ok(if source.is_absolute() {
            source.clone()
        } else {
            dir.join(source)
        })
    }

    pub fn transcript_path(&self, dir: &Path) -> Option<PathBuf> {
        self.transcript.as_ref().map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                dir.join(path)
            }
        })
    }
}

/// Write a deterministic mono 16 kHz PCM16 silence WAV for synthetic fixtures.
pub fn write_synthetic_wav(path: &Path, seconds: f64) -> Result<(), LabelError> {
    let sample_count = (seconds * 16_000.0).round();
    if !sample_count.is_finite() || sample_count < 1.0 || sample_count > 100_000_000.0 {
        return Err(LabelError::Invalid("synthetic duration".into()));
    }
    let sample_count = sample_count as u32;
    let data_bytes = u64::from(sample_count) * 2;
    let riff_bytes = 36u64
        .checked_add(data_bytes)
        .ok_or_else(|| LabelError::Invalid("synthetic size".into()))?;
    if riff_bytes > u64::from(u32::MAX) {
        return Err(LabelError::Invalid("synthetic size".into()));
    }
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(riff_bytes as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&(16_000u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    out.resize(44 + data_bytes as usize, 0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(LabelError::Io)?;
    }
    std::fs::write(path, out).map_err(LabelError::Io)
}

/// Lowercase SHA-256 of a file, streamed in bounded chunks.
pub fn sha256_file(path: &Path) -> Result<String, LabelError> {
    let mut file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LabelError::MissingSource(path.to_path_buf())
        } else {
            LabelError::Io(error)
        }
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(LabelError::Io)?;
        if count == 0 {
            break;
        }
        let chunk = buffer
            .get(..count)
            .ok_or_else(|| LabelError::Invalid("hash read bounds".into()))?;
        hasher.update(chunk);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// Verify the label's source identity before any pipeline work.
pub fn verify_hash(label: &LabelFile, source: &Path) -> Result<String, LabelError> {
    let actual = sha256_file(source)?;
    let expected = label.expected_hash().to_ascii_lowercase();
    if actual != expected {
        return Err(LabelError::HashMismatch { expected, actual });
    }
    Ok(actual)
}

pub fn ticks_from_seconds(seconds: f64) -> Result<Tick, LabelError> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(LabelError::Invalid("duration".into()));
    }
    Tick::from_seconds_f64(seconds, Rounding::Nearest).map_err(LabelError::Time)
}

pub const ONE_SECOND: Tick = Tick(TICKS_PER_SECOND);

#[derive(Debug, thiserror::Error)]
pub enum LabelError {
    #[error("label I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("label JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("label time is invalid: {0}")]
    Time(#[from] time::TimeError),
    #[error("label schema version {0} is unsupported")]
    Version(u32),
    #[error("label hash is missing or malformed")]
    MissingHash,
    #[error("label source is missing: {0}")]
    MissingSource(PathBuf),
    #[error("label source hash mismatch: expected {expected}, actual {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("label is invalid: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label_json(hash: &str) -> String {
        format!(
            r#"{{"version":1,"source":"source.bin","sha256":"{hash}","language":"en","clips":[{{"id":"a","priority":"must","segments":[{{"start":1,"end":2}}]}}]}}"#
        )
    }

    #[test]
    fn missing_hash_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("label.json");
        std::fs::write(&path, label_json("")).unwrap();
        assert!(matches!(
            LabelFile::load(&path),
            Err(LabelError::MissingHash)
        ));
    }

    #[test]
    fn mismatched_and_missing_source_hashes_are_typed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        std::fs::write(&source, b"hello").unwrap();
        let path = dir.path().join("label.json");
        std::fs::write(&path, label_json(&"0".repeat(64))).unwrap();
        let (label, label_dir) = LabelFile::load(&path).unwrap();
        let resolved = label.source_path(&label_dir, dir.path(), "label").unwrap();
        assert!(matches!(
            verify_hash(&label, &resolved),
            Err(LabelError::HashMismatch { .. })
        ));
        std::fs::remove_file(&source).unwrap();
        assert!(matches!(
            verify_hash(&label, &resolved),
            Err(LabelError::MissingSource(_))
        ));
    }
}

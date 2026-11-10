//! Transcript cache: the clip pipeline's fixed input, keyed by source content.
use crate::budget::Budget;
use crate::label::{LabelFile, sha256_bytes};
use crate::{Error, Result, asr};
use captions::{LanguageTag, Transcript};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct CachedTranscript {
    pub transcript: Transcript,
    pub cache_path: PathBuf,
    pub cache_hit: bool,
    pub source: &'static str,
    pub cost_micros: u64,
    pub wall_ms: u64,
    pub notes: Vec<String>,
}

pub struct LoadContext<'a> {
    pub label: &'a LabelFile,
    pub label_dir: &'a Path,
    pub source_sha: &'a str,
    pub out_dir: &'a Path,
    pub source: &'a Path,
    pub language: &'a LanguageTag,
    pub transcribe: bool,
}

/// Load a label-provided transcript, an existing prepared cache, or opt-in live
/// ASR. The cache is keyed by both source content and transcript content so a
/// rerun cannot silently measure a different transcript.
pub fn load(context: &LoadContext<'_>, budget: &mut Budget) -> Result<CachedTranscript> {
    let LoadContext {
        label,
        label_dir,
        source_sha,
        out_dir,
        source,
        language,
        transcribe,
    } = *context;
    let cache_dir = out_dir.join("cache/transcripts");
    if let Some(path) = label.transcript_path(label_dir) {
        let bytes = std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::MissingTranscript(path.display().to_string())
            } else {
                Error::Io(error)
            }
        })?;
        let digest = sha256_bytes(&bytes);
        let cache_path = cache_dir.join(format!("{source_sha}-{digest}.json"));
        if cache_path.exists() {
            let transcript = read_transcript(&cache_path)?;
            validate_language(language, &transcript)?;
            return Ok(cached(transcript, cache_path, true, "cache"));
        }
        let transcript: Transcript = serde_json::from_slice(&bytes)?;
        validate_language(language, &transcript)?;
        write_cache(&cache_path, &transcript)?;
        return Ok(cached(transcript, cache_path, false, "label"));
    }
    let cache_path = cache_dir.join(format!("{source_sha}.json"));
    if cache_path.exists() {
        let transcript = read_transcript(&cache_path)?;
        validate_language(language, &transcript)?;
        return Ok(cached(transcript, cache_path, true, "cache"));
    }
    let asr_path = asr::cache_path(out_dir, source_sha);
    if asr_path.exists() {
        let transcript = read_transcript(&asr_path)?;
        validate_language(language, &transcript)?;
        return Ok(cached(transcript, asr_path, true, "cache"));
    }
    if transcribe {
        let prepared = asr::prepare(source, language, source_sha, budget)?;
        let cache_path = asr::cache_path(out_dir, source_sha);
        write_cache(&cache_path, &prepared.transcript)?;
        return Ok(CachedTranscript {
            transcript: prepared.transcript,
            cache_path,
            cache_hit: false,
            source: "asr",
            cost_micros: prepared.cost_micros,
            wall_ms: prepared.wall_ms,
            notes: prepared.notes,
        });
    }
    Err(Error::MissingTranscript(format!(
        "{} (no transcript field and no prepared cache; pass --transcribe for live Scribe)",
        label_dir.display()
    )))
}

fn cached(
    transcript: Transcript,
    cache_path: PathBuf,
    cache_hit: bool,
    source: &'static str,
) -> CachedTranscript {
    CachedTranscript {
        transcript,
        cache_path,
        cache_hit,
        source,
        cost_micros: 0,
        wall_ms: 0,
        notes: Vec::new(),
    }
}

fn read_transcript(path: &Path) -> Result<Transcript> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_cache(path: &Path, transcript: &Transcript) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(transcript)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn validate_language(language: &LanguageTag, transcript: &Transcript) -> Result<()> {
    if &transcript.source_lang != language {
        return Err(Error::TranscriptLanguage {
            expected: language.as_str().to_owned(),
            actual: transcript.source_lang.as_str().to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::Settings;
    use captions::{Fingerprint, NfcText, Segment};
    use time::{SourceRange, TICKS_PER_SECOND, Tick};

    fn fixture() -> Transcript {
        Transcript {
            source_lang: LanguageTag::parse("en").unwrap(),
            fingerprint: Fingerprint {
                provider: "fixture".into(),
                model: "fixture".into(),
                settings: "fixture".into(),
            },
            segments: vec![Segment {
                id: "s0".into(),
                range: SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)).unwrap(),
                text: NfcText::from("hello."),
                words: Vec::new(),
                spans: None,
            }],
        }
    }

    #[test]
    fn a_label_transcript_is_cached_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let label_dir = dir.path().join("labels");
        std::fs::create_dir_all(&label_dir).unwrap();
        let transcript_path = label_dir.join("video.transcript.json");
        std::fs::write(&transcript_path, serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let source = label_dir.join("video.bin");
        std::fs::write(&source, b"x").unwrap();
        let label = LabelFile {
            version: 1,
            source: Some("video.bin".into()),
            sha256: "0".repeat(64),
            language: "en".into(),
            transcript: Some("video.transcript.json".into()),
            synthetic: None,
            settings: Settings::default(),
            clips: Vec::new(),
        };
        let language = LanguageTag::parse("en").unwrap();
        let context = LoadContext {
            label: &label,
            label_dir: &label_dir,
            source_sha: "abc",
            out_dir: dir.path(),
            source: &source,
            language: &language,
            transcribe: false,
        };
        let mut budget = Budget::new(0);
        let first = load(&context, &mut budget).unwrap();
        assert!(!first.cache_hit);
        assert_eq!(first.source, "label");
        let second = load(&context, &mut budget).unwrap();
        assert!(second.cache_hit);
        assert_eq!(second.source, "cache");
    }
}

//! Model metadata and integrity policy. Engine owns transfers and cache promotion.
use crate::{Error, ExecutionHooks};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf};

/// Public catalog data, validated before consent or IO. No local path is admitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub license: String,
    pub license_url: String,
    pub source: String,
    pub languages: Vec<captions::LanguageTag>,
    pub files: Vec<ModelFile>,
}
fn https(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
    })
}
impl ModelFile {
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty()
            || self.name == "."
            || self.name == ".."
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || !https(&self.url)
            || self.size == 0
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid("model file metadata"));
        }
        Ok(())
    }
}
impl Model {
    pub fn validate(&self) -> Result<(), Error> {
        if self.id.trim().is_empty()
            || self.license.trim().is_empty()
            || !https(&self.license_url)
            || !https(&self.source)
            || self.files.is_empty()
            || self.languages.is_empty()
        {
            return Err(Error::Invalid("model metadata"));
        }
        let mut names = std::collections::BTreeSet::new();
        for file in &self.files {
            file.validate()?;
            if !names.insert(&file.name) {
                return Err(Error::Invalid("duplicate model file"));
            }
        }
        Ok(())
    }
    /// Only these weight licences are eligible for a suggested default.
    /// Every download, including defaults, still requires explicit approval.
    pub fn default_eligible(&self) -> bool {
        matches!(
            self.license.as_str(),
            "MIT" | "Apache-2.0" | "BSD-2-Clause" | "BSD-3-Clause"
        )
    }
}
/// Separate from upload Consent. Composition calls grant only after a user click
/// showing the exact model licence/source/files/bytes; never serialize this token.
#[derive(Clone)]
pub struct DownloadConsent(Model);
impl std::fmt::Debug for DownloadConsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DownloadConsent([redacted])")
    }
}
impl DownloadConsent {
    pub fn grant(model: &Model) -> Result<Self, Error> {
        model.validate()?;
        Ok(Self(model.clone()))
    }
    pub fn authorize(&self, model: &Model) -> Result<(), Error> {
        model.validate()?;
        if self.0 != *model {
            return Err(Error::ConsentRequired);
        }
        Ok(())
    }
}
/// Read and hash with a fixed 64 KiB buffer, at most expected size + one byte.
/// Engine must call before atomic promotion and keep promoted files immutable.
/// On error no bytes are promoted here; callers retain their own resume checkpoint.
pub fn verify_file(
    reader: &mut dyn Read,
    file: &ModelFile,
    hooks: &dyn ExecutionHooks,
) -> Result<(), Error> {
    file.validate()?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        let remaining = file.size.saturating_sub(bytes).saturating_add(1);
        let count =
            usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| Error::Payload)?;
        let n = match reader.read(buffer.get_mut(..count).ok_or(Error::Payload)?) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            r => r.map_err(storage_error)?,
        };
        if n == 0 {
            break;
        }
        bytes = bytes.checked_add(n as u64).ok_or(Error::Payload)?;
        if bytes > file.size {
            return Err(Error::ModelIntegrity);
        }
        hash.update(buffer.get(..n).ok_or(Error::Payload)?);
    }
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    let digest: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if bytes != file.size || digest != file.sha256 {
        return Err(Error::ModelIntegrity);
    }
    Ok(())
}
/// Safe storage categories: never include paths or arbitrary OS diagnostics.
pub fn storage_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::StorageFull {
        Error::StorageFull
    } else {
        Error::ModelStorage(error.kind())
    }
}
/// OS cache location only. Does not create directories, download, or evict files.
pub fn cache_dir() -> Result<PathBuf, Error> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base: Option<PathBuf> = None;
    let base = base.filter(|p| p.is_absolute()).ok_or(Error::Unsupported)?;
    Ok(base.join("VibeCut").join("models"))
}
/// Pinned local models; metadata never starts an automatic download.
pub fn catalog() -> Result<Vec<Model>, Error> {
    let mut models = vec![Model {
        id: "whisper-large-v3-turbo".into(), license: "MIT".into(),
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE".into(),
        source: "https://huggingface.co/ggerganov/whisper.cpp".into(),
        languages: ["vi", "en", "zh"].into_iter().map(captions::LanguageTag::parse)
            .collect::<Result<_, _>>()?,
        files: vec![ModelFile {
            name: "ggml-large-v3-turbo.bin".into(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo.bin".into(),
            sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69".into(),
            size: 1_624_555_275,
        }],
    }];
    models.push(Model {
        id: "gipformer-68m-rnnt-int8".into(), license: "MIT".into(),
        license_url: "https://huggingface.co/g-group-ai-lab/gipformer-68M-rnnt/blob/318322b009200f8e77a3751fa64638e5cc85a7dc/README.md".into(),
        source: "https://huggingface.co/g-group-ai-lab/gipformer-68M-rnnt".into(),
        languages: vec![captions::LanguageTag::parse("vi")?],
        files: vec![
            ModelFile { name: "encoder.int8.onnx".into(),
                url: "https://huggingface.co/g-group-ai-lab/gipformer-68M-rnnt/resolve/318322b009200f8e77a3751fa64638e5cc85a7dc/encoder.int8.onnx".into(),
                sha256: "3cc1a719f04d6051210e03833eef0d42939352c6f4704b67c34655ceecb98809".into(), size: 70876359 },
            ModelFile { name: "decoder.int8.onnx".into(),
                url: "https://huggingface.co/g-group-ai-lab/gipformer-68M-rnnt/resolve/318322b009200f8e77a3751fa64638e5cc85a7dc/decoder.int8.onnx".into(),
                sha256: "2b6235b8a8be57b5ba57024f6119398a1aace6aff23347a04d59c9bd2686c1f6".into(), size: 1308690 },
            ModelFile { name: "joiner.int8.onnx".into(),
                url: "https://huggingface.co/g-group-ai-lab/gipformer-68M-rnnt/resolve/318322b009200f8e77a3751fa64638e5cc85a7dc/joiner.int8.onnx".into(),
                sha256: "eb74e2ead853d64161ee7af2aa2bce5fcc64752d07f72e0683926b238296d58f".into(), size: 1033417 },
            ModelFile { name: "tokens.txt".into(),
                url: "https://huggingface.co/g-group-ai-lab/gipformer-68M-rnnt/resolve/318322b009200f8e77a3751fa64638e5cc85a7dc/tokens.txt".into(),
                sha256: "f536d03c2e95ebd2930cf0abec88e823bd17d3c1933da7ae6a82db3b80605e15".into(), size: 25847 },
        ],
    });
    models.push(alignment_model()?);
    for model in &models {
        model.validate()?;
    }
    Ok(models)
}

/// Suggested model metadata only; callers still show a choice and obtain download
/// consent. No execution fallback or automatic provider selection occurs here.
pub fn suggested_transcriber(language: &captions::LanguageTag) -> Result<Model, Error> {
    const PROFILES: &[(&str, &str)] = &[("vi", "gipformer-68m-rnnt-int8")];
    let id = PROFILES
        .iter()
        .find(|(tag, _)| *tag == language.as_str())
        .map_or("whisper-large-v3-turbo", |(_, id)| *id);
    catalog()?
        .into_iter()
        .find(|m| m.id == id && m.default_eligible())
        .ok_or(Error::Unsupported)
}

/// Pinned multilingual Candle alignment checkpoint; metadata never starts a transfer.
pub fn alignment_model() -> Result<Model, Error> {
    Ok(Model {
        id: "whisper-small-dtw".into(), license: "Apache-2.0".into(),
        license_url: "https://huggingface.co/openai/whisper-small/blob/973afd24965f72e36ca33b3055d56a652f456b4d/README.md".into(),
        source: "https://huggingface.co/openai/whisper-small".into(),
        languages: ["vi", "en", "zh", "zh-Hans", "zh-Hant"].into_iter().map(captions::LanguageTag::parse).collect::<Result<_,_>>()?,
        files: vec![
            ModelFile { name: "config.json".into(), url: "https://huggingface.co/openai/whisper-small/resolve/973afd24965f72e36ca33b3055d56a652f456b4d/config.json".into(), sha256: "e6a2b489da1b5aed65a8eb8d1e7466fa867ad5643a8bc138ba708bd56b2875c4".into(), size: 1967 },
            ModelFile { name: "generation_config.json".into(), url: "https://huggingface.co/openai/whisper-small/resolve/973afd24965f72e36ca33b3055d56a652f456b4d/generation_config.json".into(), sha256: "71565b8ef50d0bf7a1193ed4bbed195b94e70c18894d81bba2f1233dcec3ab53".into(), size: 3868 },
            ModelFile { name: "tokenizer.json".into(), url: "https://huggingface.co/openai/whisper-small/resolve/973afd24965f72e36ca33b3055d56a652f456b4d/tokenizer.json".into(), sha256: "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566".into(), size: 2480466 },
            ModelFile { name: "model.safetensors".into(), url: "https://huggingface.co/openai/whisper-small/resolve/973afd24965f72e36ca33b3055d56a652f456b4d/model.safetensors".into(), sha256: "1d7734884874f1a1513ed9aa760a4f8e97aaa02fd6d93a3a85d27b2ae9ca596b".into(), size: 966995080 },
        ],
    })
}

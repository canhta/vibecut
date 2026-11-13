//! Generates the low-resolution preview proxy of one source into the file cache.
use super::{Error as ArtifactError, Result as ArtifactResult};
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState, ResourceClass,
};
use media::{ContentHash, ProxyLimits, ProxyReport, SourceAsset, StreamKind};
use std::path::PathBuf;

const PROVIDER: &str = "vibecut";
const MODEL: &str = "proxy";
const VERSION: &str = "1";
const MAGIC: &[u8; 8] = b"VPROXY01";
const MANIFEST_BYTES: usize = 32;
/// Copies of one source frame held at once: the decoded frame, its CPU copy and the
/// scaler scratch, each counted at RGBA size.
const FRAME_COPIES: u64 = 3;
/// Decoder and encoder state beside the frames.
const CODEC_BYTES: u64 = 32 << 20;

/// Canonical key of a source's proxy: its content hash and the raster edge, so
/// changed content or a different proxy size misses every older file.
pub fn proxy_key(content: ContentHash, max_edge: u32) -> ArtifactResult<CacheKey> {
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Proxy,
        source: content.0,
        provider: PROVIDER,
        model: MODEL,
        version: VERSION,
        settings: &max_edge.to_le_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .map_err(ArtifactError::from)
}

/// The proxy of one video source at the app's proxy size.
#[derive(Clone)]
pub struct ProxyRequest {
    asset: SourceAsset,
    max_edge: u32,
}

/// A proxy the cache holds after `finish`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProxyReady {
    pub key: CacheKey,
    /// False when the cache already held it and the source was not opened.
    pub generated: bool,
}

/// Retained runner job. Only `finish` publishes the staged file; a cancelled or
/// failed job removes it and leaves the cache untouched.
pub struct ProxyJob {
    pub id: JobId,
    pub cancel: CancelToken,
    key: CacheKey,
    staged: PathBuf,
}

impl ProxyRequest {
    /// Admit a video source. Still images and audio-only sources have no proxy.
    pub fn new(asset: SourceAsset, max_edge: u32) -> crate::session::Result<Self> {
        if max_edge < 2
            || asset.is_image()
            || !asset
                .probe()
                .streams
                .iter()
                .any(|s| s.kind == StreamKind::Video && s.frame_rate.is_some())
        {
            return Err(crate::session::EditError::Unsupported(
                "proxy needs a video source with a frame rate",
            ));
        }
        Ok(Self { asset, max_edge })
    }

    pub fn key(&self) -> ArtifactResult<CacheKey> {
        proxy_key(self.asset.content_hash(), self.max_edge)
    }

    /// Decode memory sized by the source frame, so a job fits the shared runner budget
    /// unless the source itself is too large for it.
    fn decode_bytes(&self) -> u64 {
        let frame = self
            .asset
            .probe()
            .streams
            .iter()
            .find_map(|stream| stream.video.as_ref())
            .map_or(u64::MAX, |video| {
                u64::from(video.width)
                    .saturating_mul(u64::from(video.height))
                    .saturating_mul(4)
            });
        frame
            .saturating_mul(FRAME_COPIES)
            .saturating_add(CODEC_BYTES)
    }

    /// Enqueue one CPU job unless the cache already holds the proxy, in which
    /// case the job completes without opening the source. The file is staged in
    /// the cache directory and bounded by half the aggregate byte limit.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> ArtifactResult<ProxyJob> {
        let key = self.key()?;
        let warm = match cache.file(key) {
            Ok(held) => held.is_some(),
            Err(crate::RuntimeError::CacheCorrupt) => false,
            Err(error) => return Err(error.into()),
        };
        let staged = cache.stage_path(key);
        let limits = ProxyLimits {
            max_edge: self.max_edge,
            max_bytes: cache.byte_budget().snapshot()?.limit / 2,
        };
        let source = self.asset.path().to_path_buf();
        let task_staged = staged.clone();
        let spec = JobSpec {
            kind: JobKind::GenerateProxy,
            class: ResourceClass::Cpu,
            input: key,
            bytes: JobBytes {
                decoded_frames: self.decode_bytes(),
                result: MANIFEST_BYTES as u64,
                ..Default::default()
            },
        };
        let (id, cancel) = runner.submit(spec, move |context| -> ArtifactResult<Vec<u8>> {
            context.check_cancelled()?;
            if warm {
                return Ok(manifest(None));
            }
            context.progress(JobStage::Encode, 0.0)?;
            let _ = std::fs::remove_file(&task_staged);
            let outcome = media::write_proxy(
                &source,
                &task_staged,
                limits,
                context.media_cancel_flag(),
                |done| {
                    context.progress(JobStage::Encode, done).ok();
                },
            )
            .map_err(ArtifactError::from)
            .and_then(|report| {
                context.check_cancelled()?;
                Ok(report)
            });
            match outcome {
                Ok(report) => {
                    context.progress(JobStage::Encode, 1.0)?;
                    Ok(manifest(Some(report)))
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&task_staged);
                    Err(error)
                }
            }
        })?;
        Ok(ProxyJob {
            id,
            cancel,
            key,
            staged,
        })
    }
}

fn manifest(report: Option<ProxyReport>) -> Vec<u8> {
    let report = report.unwrap_or(ProxyReport {
        width: 0,
        height: 0,
        frames: 0,
        bytes: 0,
    });
    let mut bytes = Vec::with_capacity(MANIFEST_BYTES);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&report.width.to_le_bytes());
    bytes.extend_from_slice(&report.height.to_le_bytes());
    bytes.extend_from_slice(&report.frames.to_le_bytes());
    bytes.extend_from_slice(&report.bytes.to_le_bytes());
    bytes
}

/// Whether a retained manifest describes a freshly written proxy; `None` for an
/// unusable manifest.
fn generated(bytes: &[u8]) -> Option<bool> {
    if bytes.len() != MANIFEST_BYTES || bytes.get(..8) != Some(MAGIC.as_slice()) {
        return None;
    }
    let frames = u64::from_le_bytes(bytes.get(16..24)?.try_into().ok()?);
    Some(frames > 0)
}

impl ProxyJob {
    pub fn key(&self) -> CacheKey {
        self.key
    }

    /// Publish the staged file through the cache once the job completed. A
    /// pending job returns `None`; cancellation and failure remove the staged
    /// file and return their typed error.
    pub fn finish(
        &self,
        runner: &mut JobRunner,
        cache: &mut ArtifactCache,
    ) -> ArtifactResult<Option<ProxyReady>> {
        let snapshot = runner
            .snapshot(self.id)
            .ok_or(ArtifactError::Runtime(crate::RuntimeError::JobNotRetained))?;
        match snapshot.state {
            JobState::Cancelled => {
                self.discard();
                Err(ArtifactError::Runtime(crate::RuntimeError::Cancelled))
            }
            JobState::Failed => {
                self.discard();
                Err(ArtifactError::Runtime(crate::RuntimeError::Failed(
                    snapshot
                        .error
                        .ok_or(ArtifactError::from(crate::SessionError::StaleResult))?,
                )))
            }
            JobState::Completed => {
                let bytes = runner
                    .output_bytes(self.id)
                    .ok_or(ArtifactError::Runtime(crate::RuntimeError::JobNotRetained))?;
                let Some(generated) = generated(bytes) else {
                    self.discard();
                    return Err(crate::RuntimeError::CacheCorrupt.into());
                };
                if generated {
                    cache.publish_file(self.key, &self.staged)?;
                }
                runner
                    .take_output(self.id)
                    .ok_or(ArtifactError::Runtime(crate::RuntimeError::JobNotRetained))?;
                Ok(Some(ProxyReady {
                    key: self.key,
                    generated,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Remove the staged file of a job that will not be published.
    pub fn discard(&self) {
        let _ = std::fs::remove_file(&self.staged);
    }
}

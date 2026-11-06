//! Transfers sound audio through bounded jobs.
use super::{
    Arc, ArtifactKind, AtomicBool, Busy, CacheKey, CachedSound, CancelToken, DownloadTransport,
    EditError, Error, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, KeyInputs,
    MAX_AUDIO_BYTES, NEXT, Ordering, Read, ResourceClass, Sound, SoundDownloads, SoundLibrary,
    Write, download_redirect, fs, retry_after, url,
};
pub(super) fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        None
    } else {
        value.parse().ok()
    }
}
impl SoundDownloads {
    pub fn new(library: Arc<SoundLibrary>, transport: Arc<dyn DownloadTransport>) -> Self {
        Self {
            library,
            transport,
            max_bytes: MAX_AUDIO_BYTES,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Production composition helper. Downloads carry no credential.
    pub fn ureq(library: Arc<SoundLibrary>) -> Self {
        Self::new(library, Arc::new(ai::transport::UreqTransport::new()))
    }
    pub fn library(&self) -> &Arc<SoundLibrary> {
        &self.library
    }
    /// Submit one cache download through the shared runner. There is one active
    /// download per library; the runner owns retries, backoff and queue bounds.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        source: &str,
    ) -> crate::Result<(JobId, CancelToken)> {
        if !url(source) {
            return Err(Error::Invalid("sound URL").into());
        }
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EditError::CacheBusy)?;
        let guard = Busy(self.busy.clone());
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::SoundAudio,
            source: *blake3::hash(source.as_bytes()).as_bytes(),
            provider: "freesound",
            model: "preview",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let worker = SoundDownloads {
            library: self.library.clone(),
            transport: self.transport.clone(),
            max_bytes: self.max_bytes,
            busy: Arc::new(AtomicBool::new(false)),
        };
        let source = source.to_owned();
        runner.submit(
            JobSpec {
                kind: JobKind::SoundDownload,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes::default(),
            },
            move |ctx| {
                let _guard = &guard;
                ctx.check_cancelled()?;
                match worker.fetch(&source, &|| ctx.cancelled(), Some(ctx), &|done, total| {
                    let fraction = total
                        .filter(|total| *total > 0)
                        .map_or(0.0, |total| (done as f64 / total as f64) as f32);
                    ctx.progress(JobStage::Download, fraction).ok();
                }) {
                    Ok(_) => Ok(Vec::new()),
                    Err(Error::Network) => Err(EditError::NetworkTransient),
                    Err(error) => Err(error.into()),
                }
            },
        )
    }
    /// Download through bounded HTTPS redirects into the library cache. Cancellation
    /// is checked before every read and write and leaves no partial cache entry.
    pub fn download(
        &self,
        source: &str,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Sound<CachedSound> {
        self.fetch(source, cancelled, None, progress)
    }
    fn fetch(
        &self,
        source: &str,
        cancelled: &dyn Fn() -> bool,
        fence: Option<&crate::jobs::JobContext>,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Sound<CachedSound> {
        if !url(source) {
            return Err(Error::Invalid("sound URL"));
        }
        let mut target = source.to_owned();
        for redirect in 0..=5 {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let reply = self.transport.get(&target, &[]).map_err(Error::from)?;
            if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
                if redirect == 5 {
                    return Err(Error::InvalidResponse);
                }
                let location = header(&reply.headers, "location").ok_or(Error::InvalidResponse)?;
                target = download_redirect(&target, location).map_err(Error::from)?;
                continue;
            }
            match reply.status {
                200 => {}
                401 | 403 => return Err(Error::Auth),
                402 => return Err(Error::Quota),
                404 => return Err(Error::InvalidResponse),
                429 => {
                    return Err(Error::RateLimit {
                        retry_after_seconds: retry_after(&reply.headers),
                    });
                }
                408 | 500..=599 => return Err(Error::Network),
                _ => return Err(Error::InvalidResponse),
            }
            let expected = match header(&reply.headers, "content-length") {
                Some(value) => {
                    let size = decimal(value).ok_or(Error::InvalidResponse)?;
                    if size == 0 || size > self.max_bytes {
                        return Err(Error::Limit);
                    }
                    Some(size)
                }
                None => None,
            };
            let temp = self.library.audio.join(format!(
                ".part-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let result = (|| {
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&temp)?;
                let mut body = reply.body;
                let mut total = 0u64;
                let mut buffer = [0u8; 65536];
                loop {
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let read = match body.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => return Err(Error::Network),
                    };
                    total = total.checked_add(read as u64).ok_or(Error::Limit)?;
                    if total > self.max_bytes {
                        return Err(Error::Limit);
                    }
                    file.write_all(buffer.get(..read).ok_or(Error::Limit)?)?;
                    progress(total, expected);
                }
                if total == 0 || expected.is_some_and(|size| size != total) {
                    return Err(Error::InvalidResponse);
                }
                file.sync_all()?;
                drop(file);
                self.library.publish_temp(source, &temp, cancelled, fence)
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            return result;
        }
        Err(Error::InvalidResponse)
    }
}

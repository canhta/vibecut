//! Transfers music audio through bounded jobs.
use super::{
    Arc, ArtifactKind, CacheKey, CachedMusic, CancelToken, Deref, DownloadTransport, EngineError,
    Error, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, KeyInputs, MAX_AUDIO_BYTES,
    Music, MusicDownloads, MusicLibrary, NEXT, Operation, Ordering, Read, ResourceClass, Write,
    download_redirect, fs, url,
};
use transport::retry_after;
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
impl MusicDownloads {
    pub fn new(library: Arc<MusicLibrary>, transport: Arc<dyn DownloadTransport>) -> Self {
        Self { library, transport }
    }
    /// Production composition helper. Downloads carry no credential.
    pub fn ureq(library: Arc<MusicLibrary>) -> Self {
        Self::new(library, Arc::new(transport::UreqTransport::new()))
    }
    pub fn library(&self) -> &Arc<MusicLibrary> {
        &self.library
    }
    /// Submit under owned library admission through queue/retry/terminal/drop.
    /// Contention is `CacheBusy`; the runner owns backoff and queue bounds.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        source: &str,
    ) -> engine::control::Result<(JobId, CancelToken)> {
        if !url(source) {
            return Err(Error::Invalid("music URL").into());
        }
        let operation = Operation::acquire(self.library.clone()).map_err(|error| match error {
            Error::Project(project::Error::Busy) => {
                EngineError::Runtime(engine::RuntimeError::CacheBusy)
            }
            error => error.into(),
        })?;
        let key = CacheKey::new(KeyInputs {
            // The audio download path, retry budget and artifact bound are shared
            // with the sound library; the provider and URL keep the keys apart.
            kind: ArtifactKind::SoundAudio,
            source: *blake3::hash(source.as_bytes()).as_bytes(),
            provider: "openverse",
            model: "audio",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let transport = self.transport.clone();
        let source = source.to_owned();
        Ok(runner.submit(
            JobSpec {
                kind: JobKind::SoundDownload,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes::default(),
            },
            move |ctx| {
                ctx.check_cancelled()?;
                match operation.fetch(
                    transport.as_ref(),
                    &source,
                    &|| ctx.cancelled(),
                    Some(ctx),
                    &|done, total| {
                        let fraction = total
                            .filter(|total| *total > 0)
                            .map_or(0.0, |total| (done as f64 / total as f64) as f32);
                        ctx.progress(JobStage::Download, fraction).ok();
                    },
                ) {
                    Ok(_) => Ok(Vec::new()),
                    Err(Error::Network) => {
                        Err(EngineError::Runtime(engine::RuntimeError::NetworkTransient))
                    }
                    Err(error) => Err(error.into()),
                }
            },
        )?)
    }
    /// Hold library admission throughout a synchronous bounded transfer. Contention
    /// is Project(Busy). Cancellation checks surround reads; cleanup failures are
    /// typed and may leave an unpromoted part (see the owner contract).
    pub fn download(
        &self,
        source: &str,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Music<CachedMusic> {
        self.library
            .operation()?
            .fetch(self.transport.as_ref(), source, cancelled, None, progress)
    }
}
impl<T: Deref<Target = MusicLibrary>> Operation<T> {
    fn fetch(
        &self,
        transport: &dyn DownloadTransport,
        source: &str,
        cancelled: &dyn Fn() -> bool,
        fence: Option<&engine::jobs::JobContext>,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Music<CachedMusic> {
        if !url(source) {
            return Err(Error::Invalid("music URL"));
        }
        let mut target = source.to_owned();
        for redirect in 0..=5 {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            self.0.validate()?;
            let reply = transport.get(&target, &[]).map_err(Error::from)?;
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
                    if size == 0 || size > MAX_AUDIO_BYTES {
                        return Err(Error::Limit);
                    }
                    Some(size)
                }
                None => None,
            };
            self.0.validate()?;
            let temp = self.0.audio.path().join(format!(
                ".part-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut part = Part {
                library: &self.0,
                path: temp,
                created: false,
            };
            let result = (|| {
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&part.path)?;
                part.created = true;
                let mut body = reply.body;
                let mut total = 0u64;
                let mut buffer = [0u8; 65536];
                loop {
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let read = match body.read(&mut buffer) {
                        Ok(read) => read,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => return Err(Error::Network),
                    };
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    self.0.validate()?;
                    if read == 0 {
                        break;
                    }
                    total = total.checked_add(read as u64).ok_or(Error::Limit)?;
                    if total > MAX_AUDIO_BYTES {
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
                self.publish_temp(source, &part.path, cancelled, fence)
            })();
            match result {
                Ok(cached) => {
                    part.created = false;
                    return Ok(cached);
                }
                Err(primary) => {
                    if let Err(cleanup) = part.cleanup()
                        && !matches!(primary, Error::Project(_) | Error::Io(_))
                    {
                        return Err(cleanup);
                    }
                    return Err(primary);
                }
            }
        }
        Err(Error::InvalidResponse)
    }
}

// A path becomes owned only after create_new succeeds. Explicit cleanup reports
// failure; drop is the best-effort unwind path, never admission for a collision.
struct Part<'a> {
    library: &'a MusicLibrary,
    path: super::PathBuf,
    created: bool,
}
impl Part<'_> {
    fn cleanup(&mut self) -> Music<()> {
        if !std::mem::take(&mut self.created) {
            return Ok(());
        }
        self.library.validate()?;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
impl Drop for Part<'_> {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

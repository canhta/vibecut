//! Transfers pinned font files into durable storage.
use super::{
    ADMITTED_BY, Active, Arc, ArtifactKind, AtomicU64, BTreeSet, BUFFER, Cache, CacheKey,
    CancelToken, Digest, DownloadTransport, EditError, FontError, FontStore, GOOGLE_FONTS_COMMIT,
    GoogleFamily, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, KeyInputs,
    LICENSE_FILE, Mutex, Ordering, Path, PathBuf, Read, ResourceClass, Result, Sha256, Staging,
    WORK_BYTES, Write, fs, io, read_cache,
};
impl FontStore {
    /// Start the explicit download of a catalog family. `accepted` is the user's acknowledgement
    /// of the displayed source and licence; without it nothing starts. Files go to a private
    /// staging directory, are verified against the pinned size and SHA-256, and the finished
    /// directory is renamed into place once; a failed or cancelled job leaves no cache entry.
    ///
    /// # Errors
    ///
    /// `ConsentRequired` without acknowledgement; [`FontError::NotDownloadable`] for a family
    /// outside the catalog; [`FontError::DownloadActive`] while the same family is downloading;
    /// runner admission errors.
    pub fn submit_download(
        &self,
        runner: &mut JobRunner,
        family: &str,
        accepted: bool,
    ) -> Result<(JobId, CancelToken)> {
        if !accepted {
            return Err(engine::providers::Error::ConsentRequired.into());
        }
        let g = self
            .google_family(family.trim())
            .ok_or(FontError::NotDownloadable)?
            .clone();
        let guard = Active::acquire(&self.active, &g.slug)?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::FontDownload,
            source: [0; 32],
            provider: "fonts",
            model: &g.slug,
            version: GOOGLE_FONTS_COMMIT,
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let root = self.root.clone();
        let transport = self.transport.clone();
        Ok(runner.submit::<engine::EditError>(
            JobSpec {
                kind: JobKind::DownloadFont,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes {
                    compositor: WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                let _hold = &guard;
                download(&root, &g, transport.as_ref(), ctx)?;
                Ok(Vec::new())
            },
        )?)
    }
}
impl Active {
    pub(super) fn acquire(set: &Arc<Mutex<BTreeSet<String>>>, slug: &str) -> Result<Self> {
        let mut held = set.lock().map_err(|_| engine::RuntimeError::Poisoned)?;
        if !held.insert(slug.to_owned()) {
            return Err(FontError::DownloadActive.into());
        }
        Ok(Self {
            set: set.clone(),
            slug: slug.to_owned(),
        })
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut held) = self.set.lock() {
            held.remove(&self.slug);
        }
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

pub(super) fn sidecar(font: &Path) -> PathBuf {
    let mut path = font.as_os_str().to_os_string();
    path.push(".attribution");
    PathBuf::from(path)
}
impl Staging {
    pub(super) fn create(root: &Path, slug: &str) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(root)?;
        let path = root.join(format!(
            ".partial-{slug}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            published: false,
        })
    }
    pub(super) fn publish(mut self, target: &Path) -> Result<()> {
        match fs::symlink_metadata(target) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(target)?,
            Ok(_) => fs::remove_file(target)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::rename(&self.path, target)?;
        self.published = true;
        Ok(())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn download(
    root: &Path,
    g: &GoogleFamily,
    http: &dyn DownloadTransport,
    ctx: &JobContext,
) -> Result<()> {
    ctx.check_cancelled()?;
    let target = root.join(&g.slug);
    if matches!(read_cache(root, g)?, Cache::Valid(_)) {
        return Ok(());
    }
    let staging = Staging::create(root, &g.slug)?;
    let total = g.download_bytes().max(1);
    let mut done = 0u64;
    ctx.progress(JobStage::Download, 0.0)?;
    let items = g
        .files
        .iter()
        .map(|f| (f.name.as_str(), f.url.as_str(), f.size, f.sha256.as_str()))
        .chain(std::iter::once((
            LICENSE_FILE,
            g.license_url.as_str(),
            g.license_size,
            g.license_sha256.as_str(),
        )));
    for (name, url, size, sha256) in items {
        fetch(
            http,
            url,
            size,
            sha256,
            &staging.path.join(name),
            ctx,
            &mut done,
            total,
        )?;
    }
    for file in &g.files {
        let record = format!(
            "asset: {name}\ntitle: {family} ({name})\nauthor: {author}\nsource: {url}\nlicense: {license}\nlicense-file: {LICENSE_FILE}\nadded: {ADMITTED_BY}\nnotes: Downloaded on explicit user request from {page}; SHA-256 {sha}. Verified against the catalog pin before use.\n",
            name = file.name,
            family = g.family,
            author = g.author,
            url = file.url,
            license = g.license,
            page = g.source,
            sha = file.sha256,
        );
        fs::write(sidecar(&staging.path.join(&file.name)), record)?;
    }
    ctx.progress(JobStage::Verify, 1.0)?;
    ctx.commit_publication(|| staging.publish(&target))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fetch(
    http: &dyn DownloadTransport,
    url: &str,
    size: u64,
    sha256: &str,
    path: &Path,
    ctx: &JobContext,
    done: &mut u64,
    total: u64,
) -> Result<()> {
    let headers = [("Accept-Encoding".to_owned(), "identity".to_owned())];
    let reply = engine::transfer::response(http, url, &headers, ctx)?;
    if reply.status != 200 {
        return Err(FontError::Response.into());
    }
    let mut file = fs::File::create_new(path)?;
    let mut body = reply.body.take(size.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    let mut written = 0u64;
    loop {
        ctx.check_cancelled()?;
        let read = body
            .read(&mut buffer)
            .map_err(|_| EditError::Runtime(engine::RuntimeError::NetworkTransient))?;
        if read == 0 {
            break;
        }
        let chunk = buffer
            .get(..read)
            .ok_or(EditError::Control(engine::control::ControlError::Limit))?;
        written += read as u64;
        if written > size {
            return Err(FontError::Integrity.into());
        }
        hasher.update(chunk);
        file.write_all(chunk)?;
        *done += read as u64;
        ctx.progress(JobStage::Download, (*done as f32 / total as f32).min(1.0))?;
    }
    if written != size || hex(&hasher.finalize()) != sha256 {
        return Err(FontError::Integrity.into());
    }
    file.sync_all()?;
    Ok(())
}

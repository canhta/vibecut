//! Transfers one pinned collection into durable storage on explicit user action.
use super::collections::{Collection, LICENSE_FILE};
use super::sanitize::check_svg;
use super::store::{ADMITTED_BY, StickerStore, shape_matches, sidecar_path};
use engine::StickerError;
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::control::{Error as EngineError, Result};
use engine::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use transport::DownloadTransport;

const BUFFER: usize = 32 * 1024;
/// Job reservation: the streaming buffer, one file and hashing state.
const WORK_BYTES: u64 = 256 * 1024;

impl StickerStore {
    /// Starts the explicit download of a collection. `accepted` is the user's acknowledgement of
    /// the displayed source and licence; without it nothing starts. Every file streams into a
    /// private staging directory, is verified against its pinned size and SHA-256 and the
    /// sticker admission check, and the finished directory is renamed into place once. A failed
    /// or cancelled job leaves no cache entry. A transport that cannot reach the server fails the
    /// job with a transient network error ([`super::is_offline`]).
    ///
    /// # Errors
    ///
    /// `ConsentRequired` without acknowledgement; [`StickerError::NotDownloadable`] for an
    /// unknown collection; [`StickerError::DownloadActive`] while the same collection is
    /// downloading; runner admission errors.
    pub fn submit_download(
        &self,
        runner: &mut JobRunner,
        collection: &str,
        accepted: bool,
    ) -> Result<(JobId, CancelToken)> {
        if !accepted {
            return Err(engine::providers::Error::ConsentRequired.into());
        }
        let collection = self
            .collection(collection.trim())
            .ok_or(StickerError::NotDownloadable)?
            .clone();
        let guard = Active::acquire(&self.active, &collection.id)?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::StickerDownload,
            source: [0; 32],
            provider: "stickers",
            model: &collection.id,
            version: &collection.commit,
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let root = self.root.clone();
        let transport = self.transport.clone();
        Ok(runner.submit::<EngineError>(
            JobSpec {
                kind: JobKind::DownloadSticker,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes {
                    compositor: WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                let _hold = &guard;
                download(&root, &collection, transport.as_ref(), ctx)?;
                Ok(Vec::new())
            },
        )?)
    }
}

/// Marks a collection as downloading until dropped.
struct Active {
    set: Arc<Mutex<BTreeSet<String>>>,
    id: String,
}

impl Active {
    fn acquire(set: &Arc<Mutex<BTreeSet<String>>>, id: &str) -> Result<Self> {
        let mut held = set.lock().map_err(|_| engine::RuntimeError::Poisoned)?;
        if !held.insert(id.to_owned()) {
            return Err(StickerError::DownloadActive.into());
        }
        Ok(Self {
            set: set.clone(),
            id: id.to_owned(),
        })
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut held) = self.set.lock() {
            held.remove(&self.id);
        }
    }
}

/// Private staging directory, removed on drop unless published.
struct Staging {
    path: PathBuf,
    published: bool,
}

impl Staging {
    fn create(root: &Path, id: &str) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(root)?;
        let path = root.join(format!(
            ".partial-{id}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            published: false,
        })
    }

    fn publish(mut self, target: &Path) -> Result<()> {
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

fn download(
    root: &Path,
    collection: &Collection,
    http: &dyn DownloadTransport,
    ctx: &JobContext,
) -> Result<()> {
    ctx.check_cancelled()?;
    let target = root.join(&collection.id);
    if target.is_dir() && shape_matches(&target, collection)? {
        return Ok(());
    }
    let staging = Staging::create(root, &collection.id)?;
    let mut transfer = Transfer {
        http,
        ctx,
        done: 0,
        total: collection.download_bytes().max(1),
    };
    ctx.progress(JobStage::Download, 0.0)?;
    let license = transfer.fetch(
        &collection.raw_url(&collection.license_path),
        collection.license_size,
        &collection.license_sha256,
    )?;
    write_file(
        &staging.path,
        LICENSE_FILE,
        &license,
        &record(
            collection,
            LICENSE_FILE,
            &format!("{} licence text", collection.name),
            &collection.raw_url(&collection.license_path),
            &collection.license_sha256,
            "License text for the cached sticker files in this directory.",
        ),
    )?;
    for entry in &collection.entries {
        let url = collection.raw_url(&entry.path);
        let bytes = transfer.fetch(&url, entry.size, &entry.sha256)?;
        check_svg(&bytes)?;
        let file = format!("{}.svg", entry.value);
        write_file(
            &staging.path,
            &file,
            &bytes,
            &record(
                collection,
                &file,
                &format!("{} ({})", entry.name, collection.name),
                &url,
                &entry.sha256,
                "Unmodified SVG.",
            ),
        )?;
    }
    ctx.progress(JobStage::Verify, 1.0)?;
    ctx.commit_publication(|| staging.publish(&target))
}

fn write_file(dir: &Path, name: &str, bytes: &[u8], sidecar: &str) -> Result<()> {
    let file = dir.join(name);
    fs::write(&file, bytes)?;
    fs::write(sidecar_path(&file), sidecar)?;
    Ok(())
}

fn record(
    collection: &Collection,
    asset: &str,
    title: &str,
    url: &str,
    sha256: &str,
    note: &str,
) -> String {
    format!(
        "asset: {asset}\ntitle: {title}\nauthor: {author}\nsource: {url}\nlicense: {license}\nlicense-file: {LICENSE_FILE}\nadded: {ADMITTED_BY}\nnotes: {note} Downloaded on explicit user request from {home}; SHA-256 {sha256}. Verified against the catalog pin before use.\ncollection: {id}\ncommit: {commit}\n",
        author = collection.author,
        license = collection.license,
        home = collection.source(),
        id = collection.id,
        commit = collection.commit,
    )
}

struct Transfer<'a> {
    http: &'a dyn DownloadTransport,
    ctx: &'a JobContext,
    done: u64,
    total: u64,
}

impl Transfer<'_> {
    /// One file streamed to memory (at most the pinned size) and verified against the pin.
    fn fetch(&mut self, url: &str, size: u64, sha256: &str) -> Result<Vec<u8>> {
        let headers = [("Accept-Encoding".to_owned(), "identity".to_owned())];
        let reply = engine::transfer::response(self.http, url, &headers, self.ctx)?;
        if reply.status != 200 {
            return Err(StickerError::Response.into());
        }
        let mut body = reply.body.take(size.saturating_add(1));
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or_default());
        let mut buffer = vec![0u8; BUFFER];
        loop {
            self.ctx.check_cancelled()?;
            let read = body
                .read(&mut buffer)
                .map_err(|_| EngineError::Runtime(engine::RuntimeError::NetworkTransient))?;
            if read == 0 {
                break;
            }
            let chunk = buffer
                .get(..read)
                .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
            bytes.extend_from_slice(chunk);
            if bytes.len() as u64 > size {
                return Err(StickerError::Integrity.into());
            }
            self.done += read as u64;
            self.ctx.progress(
                JobStage::Download,
                (self.done as f32 / self.total as f32).min(1.0),
            )?;
        }
        if bytes.len() as u64 != size || super::sha256_hex(&bytes) != sha256 {
            return Err(StickerError::Integrity.into());
        }
        Ok(bytes)
    }
}

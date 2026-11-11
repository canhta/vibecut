//! Explicitly consented model transfers, through the shared runner/cache/transport.
use crate::cache::{ArtifactCache, ModelLease, NativeEdit, strong_etag};
use crate::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use crate::transfer::{header, response};
use crate::{ByteBudget, ByteOwner, EditError, Result};
use ai::models::{DownloadConsent, Model};
use ai::transport::DownloadTransport;
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
const WORK_BYTES: u64 = 256 * 1024;
const BUFFER: usize = 65536;
const CHECKPOINT_BYTES: u64 = 4 * 1024 * 1024;
/// OS disk preflight and write boundary. Errors retain their IO category; never
/// silently continue after StorageFull. Injected implementations support failure tests.
pub trait ModelIo: Send + Sync {
    fn available(&self, path: &Path) -> io::Result<u64>;
    fn write(&self, file: &mut File, bytes: &[u8]) -> io::Result<()>;
}
pub struct SystemModelIo;
impl ModelIo for SystemModelIo {
    fn available(&self, path: &Path) -> io::Result<u64> {
        fs4::available_space(path)
    }
    fn write(&self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        file.write_all(bytes)
    }
}
struct Busy(Arc<AtomicBool>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct CacheWork {
    slot: Arc<Mutex<Option<ArtifactCache>>>,
    cache: Option<ArtifactCache>,
}
impl CacheWork {
    fn take(slot: &Arc<Mutex<Option<ArtifactCache>>>) -> Result<Self> {
        let cache = slot
            .lock()
            .map_err(|_| crate::RuntimeError::Poisoned)?
            .take()
            .ok_or(EditError::CacheBusy)?;
        Ok(Self {
            slot: slot.clone(),
            cache: Some(cache),
        })
    }
    fn cache(&mut self) -> Result<&mut ArtifactCache> {
        self.cache.as_mut().ok_or(EditError::CacheBusy)
    }
}
impl Drop for CacheWork {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = self.cache.take();
        }
    }
}
/// One active transfer per owned cache; additional submissions refuse CacheBusy.
/// There is no runner/thread here: caller supplies its existing JobRunner.
/// Drop does not remove files. Installed models and checkpoints require user removal.
pub struct ModelTransfers {
    cache: Arc<Mutex<Option<ArtifactCache>>>,
    busy: Arc<AtomicBool>,
    budget: ByteBudget,
    transport: Arc<dyn DownloadTransport>,
    io: Arc<dyn ModelIo>,
}
impl ModelTransfers {
    pub fn new(
        cache: ArtifactCache,
        transport: Arc<dyn DownloadTransport>,
        io: Arc<dyn ModelIo>,
    ) -> Self {
        let budget = cache.byte_budget();
        Self {
            cache: Arc::new(Mutex::new(Some(cache))),
            busy: Arc::new(AtomicBool::new(false)),
            budget,
            transport,
            io,
        }
    }
    /// Runtime's default model path; does not download or grant consent.
    pub fn open_default(
        budget: ByteBudget,
        max_entries: usize,
        transport: Arc<dyn DownloadTransport>,
    ) -> Result<(Self, usize)> {
        let root = crate::cache::CacheRoot::discover()?.models();
        let (cache, diagnostics) = ArtifactCache::open(&root, budget.clone(), max_entries)?;
        Ok((
            Self::new(cache, transport, Arc::new(SystemModelIo)),
            diagnostics,
        ))
    }
    /// The same, over the system HTTPS transport; for composition roots that do not own `ai`.
    pub fn open_native(budget: ByteBudget, max_entries: usize) -> Result<(Self, usize)> {
        Self::open_default(
            budget,
            max_entries,
            Arc::new(ai::transport::UreqTransport::new()),
        )
    }
    /// Blocking verified cache lookup, including offline use without download consent.
    pub fn installed(
        &self,
        model: &Model,
        hooks: &dyn ai::ExecutionHooks,
    ) -> Result<Option<ModelLease>> {
        let _memory = self.budget.reserve(ByteOwner::CacheReads, WORK_BYTES)?;
        let mut work = CacheWork::take(&self.cache)?;
        work.cache()?.model(model, hooks)
    }
    /// Only the composition root's affirmative licence/source/size download action
    /// may supply this ai-owned token. Choosing a provider never calls grant here.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        model: Model,
        consent: Option<DownloadConsent>,
    ) -> Result<(JobId, CancelToken)> {
        if !runner.uses_budget(&self.budget) {
            return Err(EditError::Invalid("model and runner budgets differ"));
        }
        let consent = consent.ok_or(EditError::Ai(ai::Error::ConsentRequired))?;
        consent.authorize(&model)?;
        let key = ArtifactCache::model_key(&model)?;
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EditError::CacheBusy)?;
        let reservation = Busy(self.busy.clone());
        let slot = self.cache.clone();
        let http = self.transport.clone();
        let io = self.io.clone();
        runner.submit(
            JobSpec {
                kind: JobKind::DownloadModel,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes {
                    compositor: WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                let _hold = &reservation;
                consent.authorize(&model)?;
                ctx.check_cancelled()?;
                let mut work = CacheWork::take(&slot)?;
                download(work.cache()?, &model, http.as_ref(), io.as_ref(), ctx)?;
                Ok(Vec::new())
            },
        )
    }
    /// Called only by an explicit model-manager removal action; live native leases refuse.
    pub fn remove(&self, model: &Model) -> Result<()> {
        if self.busy.load(Ordering::Acquire) {
            return Err(EditError::CacheBusy);
        }
        let mut work = CacheWork::take(&self.cache)?;
        work.cache()?.remove_model(model)
    }
}
fn storage(error: io::Error) -> EditError {
    if error.kind() == io::ErrorKind::StorageFull {
        EditError::StorageFull
    } else {
        EditError::Io(error)
    }
}
fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        None
    } else {
        value.parse().ok()
    }
}
fn range(value: &str, start: u64, size: u64) -> bool {
    let Some((span, total)) = value.strip_prefix("bytes ").and_then(|v| v.split_once('/')) else {
        return false;
    };
    let Some((a, b)) = span.split_once('-') else {
        return false;
    };
    decimal(a) == Some(start) && decimal(b) == size.checked_sub(1) && decimal(total) == Some(size)
}
fn durable(edit: &mut NativeEdit, file: &File, index: usize, offset: u64) -> Result<()> {
    file.sync_all().map_err(storage)?;
    let old = edit
        .state
        .parts
        .get(index)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?
        .offset;
    if old == offset {
        return Ok(());
    }
    edit.state
        .parts
        .get_mut(index)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?
        .offset = offset;
    if let Err(error) = edit.checkpoint() {
        edit.state
            .parts
            .get_mut(index)
            .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?
            .offset = old;
        return Err(error);
    }
    Ok(())
}
fn reset(edit: &mut NativeEdit, file: &File, index: usize) -> Result<()> {
    let part = edit
        .state
        .parts
        .get_mut(index)
        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
    part.offset = 0;
    part.etag = None;
    edit.checkpoint()?;
    file.set_len(0).map_err(storage)?;
    edit.report_length(index, 0)
}
fn download(
    cache: &mut ArtifactCache,
    model: &Model,
    http: &dyn DownloadTransport,
    io: &dyn ModelIo,
    ctx: &JobContext,
) -> Result<()> {
    if cache.model(model, ctx)?.is_some() {
        return Ok(());
    }
    let total = model.files.iter().try_fold(0u64, |sum, file| {
        sum.checked_add(file.size).ok_or(EditError::Limit)
    })?;
    // Existing checkpoint bytes can be reused, or reclaimed before a full restart.
    let needed = cache.remaining_model(model)?;
    if io.available(cache.root_path()).map_err(storage)? < needed {
        return Err(EditError::StorageFull);
    }
    let mut edit = cache.edit_model(model)?;
    for (index, expected) in model.files.iter().enumerate() {
        ctx.check_cancelled()?;
        let part = edit
            .state
            .parts
            .get(index)
            .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?
            .clone();
        let path = edit.directory.join(&expected.name);
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(storage)?;
        let mut offset = part.offset;
        if file.metadata()?.len() < offset {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        if offset > 0 && offset < expected.size && part.etag.is_none() {
            edit.state
                .parts
                .get_mut(index)
                .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?
                .offset = 0;
            edit.checkpoint()?;
            offset = 0;
        }
        file.set_len(offset).map_err(storage)?;
        edit.report_length(index, offset)?;
        file.seek(SeekFrom::Start(offset)).map_err(storage)?;
        if offset < expected.size {
            let mut headers = vec![("Accept-Encoding".into(), "identity".into())];
            if offset > 0 {
                headers.push(("Range".into(), format!("bytes={offset}-")));
                headers.push((
                    "If-Range".into(),
                    part.etag
                        .clone()
                        .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?,
                ));
            }
            let mut reply = response(http, &expected.url, &headers, ctx)?;
            if matches!(reply.status, 408 | 429 | 500..=599) {
                return Err(EditError::Runtime(crate::RuntimeError::NetworkTransient));
            }
            if header(&reply, "content-encoding")?
                .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
            {
                return Err(EditError::ModelResponse);
            }
            let tag = header(&reply, "etag")?
                .filter(|v| strong_etag(v))
                .map(str::to_owned);
            match reply.status {
                200 => {
                    // A server ignoring Range/If-Range replaces the representation; never append.
                    offset = 0;
                    if header(&reply, "content-range")?.is_some() {
                        return Err(EditError::ModelResponse);
                    }
                }
                206 if offset > 0
                    && header(&reply, "content-range")?
                        .is_some_and(|v| range(v, offset, expected.size)) =>
                {
                    if tag != part.etag {
                        let p = edit
                            .state
                            .parts
                            .get_mut(index)
                            .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
                        p.offset = 0;
                        p.etag = None;
                        edit.checkpoint()?;
                        file.set_len(0).map_err(storage)?;
                        edit.report_length(index, 0)?;
                        return Err(EditError::Runtime(crate::RuntimeError::NetworkTransient));
                    }
                }
                _ => return Err(EditError::ModelResponse),
            }
            let remaining = expected.size - offset;
            if let Some(length) = header(&reply, "content-length")?
                && decimal(length) != Some(remaining)
            {
                return Err(EditError::ModelResponse);
            }
            {
                let p = edit
                    .state
                    .parts
                    .get_mut(index)
                    .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?;
                p.offset = offset;
                p.etag = tag;
            }
            ctx.check_cancelled()?;
            edit.checkpoint()?;
            file.set_len(offset).map_err(storage)?;
            edit.report_length(index, offset)?;
            file.seek(SeekFrom::Start(offset)).map_err(storage)?;
            let mut buffer = [0u8; BUFFER];
            loop {
                if ctx.cancelled() {
                    durable(&mut edit, &file, index, offset)?;
                    return Err(EditError::Cancelled);
                }
                let limit = usize::try_from(
                    (expected.size - offset)
                        .saturating_add(1)
                        .min(BUFFER as u64),
                )
                .map_err(|_| EditError::Limit)?;
                let read = match reply
                    .body
                    .read(buffer.get_mut(..limit).ok_or(EditError::Limit)?)
                {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        durable(&mut edit, &file, index, offset)?;
                        return Err(EditError::Runtime(crate::RuntimeError::NetworkTransient));
                    }
                };
                if read == 0 {
                    durable(&mut edit, &file, index, offset)?;
                    if offset != expected.size {
                        return Err(EditError::Runtime(crate::RuntimeError::NetworkTransient));
                    }
                    break;
                }
                if read as u64 > expected.size - offset {
                    reset(&mut edit, &file, index)?;
                    return Err(EditError::Ai(ai::Error::ModelIntegrity));
                }
                if ctx.cancelled() {
                    durable(&mut edit, &file, index, offset)?;
                    return Err(EditError::Cancelled);
                }
                edit.report_length(
                    index,
                    offset.checked_add(read as u64).ok_or(EditError::Limit)?,
                )?;
                if let Err(error) = io.write(&mut file, buffer.get(..read).ok_or(EditError::Limit)?)
                {
                    let _ = file.set_len(offset);
                    if let Ok(meta) = file.metadata() {
                        let _ = edit.report_length(index, meta.len());
                    }
                    let _ = durable(&mut edit, &file, index, offset);
                    return Err(storage(error));
                }
                offset = offset.checked_add(read as u64).ok_or(EditError::Limit)?;
                let saved = edit
                    .state
                    .parts
                    .get(index)
                    .ok_or(EditError::Runtime(crate::RuntimeError::CacheCorrupt))?
                    .offset;
                if offset - saved >= CHECKPOINT_BYTES || offset == expected.size {
                    durable(&mut edit, &file, index, offset)?;
                }
                let prior = model.files.iter().take(index).map(|f| f.size).sum::<u64>();
                ctx.progress(
                    JobStage::Download,
                    ((prior + offset) as f64 / total as f64) as f32,
                )?;
            }
        }
        file.seek(SeekFrom::Start(0)).map_err(storage)?;
        ctx.progress(JobStage::Verify, 0.0)?;
        if let Err(error) = ai::models::verify_file(&mut file, expected, ctx) {
            if matches!(error, ai::Error::ModelIntegrity) {
                reset(&mut edit, &file, index)?;
            }
            return Err(error.into());
        }
    }
    ctx.commit_publication(|| {
        edit.state.complete = true;
        edit.checkpoint()
    })
}

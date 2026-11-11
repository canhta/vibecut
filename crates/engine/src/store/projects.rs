//! Managed project catalog. External-file projects continue to use Autosave directly.
use crate::RuntimeError;
use crate::session::{
    EditError, SessionError,
    wire::{ErrorCode, Failure, JobFailurePolicy},
};
use crate::{ByteBudget, Command, NfcText, Outcome, Session};
use project::{Metadata, Project, ProjectId, Thumbnail, WriterLease};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Managed library failures preserve their lower producer and safe diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
}
pub type Result<T> = std::result::Result<T, Error>;
impl Failure for Error {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Io(_) => ErrorCode::Io,
            Self::Media(_) => ErrorCode::Media,
        }
    }
    fn transient(&self) -> bool {
        match self {
            Self::Runtime(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> JobFailurePolicy {
        match self {
            Self::Runtime(error) => error.job_policy(),
            _ => JobFailurePolicy::default(),
        }
    }
}
impl From<project::Error> for Error {
    fn from(error: project::Error) -> Self {
        Self::Edit(error.into())
    }
}
impl From<SessionError> for Error {
    fn from(error: SessionError) -> Self {
        Self::Edit(error.into())
    }
}

const PROJECT: &str = "project.vibecut";
const MAX_ENTRIES: usize = 1024;
const HISTORY: usize = 32;
/// Trashed entries keep their complete directory beside the active ones under
/// this hidden prefix; the project ID follows in lowercase hex.
const TRASH: &str = ".trash-";
/// Store-owned record of when an entry moved to Trash, inside its directory.
const TRASH_RECORD: &str = "trash.json";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// Whole days an entry may stay in Trash before the next catalog open purges it.
pub const RETENTION_DAYS: u64 = 30;

/// Listing is derived from the project and its one metadata sidecar, never an index copy.
#[derive(Clone, serde::Serialize)]
pub struct ProjectEntry {
    pub id: ProjectId,
    pub short_id: String,
    pub name: NfcText,
    pub duration: time::Tick,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
    pub thumbnail: Thumbnail,
    pub path: PathBuf,
    /// Unix milliseconds of the Trash move; `None` for active entries and for
    /// trashed entries whose record is missing, which are never purged by age.
    pub trashed_unix_ms: Option<u64>,
}

impl ProjectEntry {
    /// Whole days this trashed entry may stay before the next automatic purge.
    /// Active entries and entries without a readable record report `None`.
    pub fn trash_days_left(&self, now_unix_ms: u64) -> Option<u64> {
        let trashed = self.trashed_unix_ms?;
        let retention = RETENTION_DAYS.checked_mul(DAY_MS)?;
        let remaining = retention.saturating_sub(now_unix_ms.saturating_sub(trashed));
        Some(remaining.saturating_add(DAY_MS - 1) / DAY_MS)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TrashRecord {
    unix_ms: u64,
}

enum Step {
    Rename {
        id: ProjectId,
        before: NfcText,
        after: NfcText,
        before_hash: [u8; 32],
        after_hash: [u8; 32],
    },
    Duplicate {
        id: ProjectId,
        hash: [u8; 32],
        hidden: PathBuf,
    },
}

/// One library owns one explicitly supplied root. Its exclusive lease serializes
/// catalog mutations; project leases also refuse operations on open projects.
/// History lasts for this handle only. Delete moves the entry to Trash and clears
/// catalog history; purge is the only permanent removal.
pub struct ProjectLibrary {
    root: PathBuf,
    _lease: WriterLease,
    budget: ByteBudget,
    undo: VecDeque<Step>,
    redo: Vec<Step>,
}
impl ProjectLibrary {
    pub fn open(root: &Path, budget: ByteBudget) -> Result<Self> {
        private_directory(root, true)?;
        if !fs::symlink_metadata(root)?.file_type().is_dir() {
            return Err(Error::Edit(crate::EditError::Invalid("library root")));
        }
        let root = root.canonicalize()?;
        let lease = WriterLease::acquire(&root.join("catalog.vibecut"))?;
        WriterLease::enable_directory_moves(&root)?;
        let mut library = Self {
            root,
            _lease: lease,
            budget,
            undo: VecDeque::new(),
            redo: Vec::new(),
        };
        // Retention is a catalog policy, so expired Trash entries leave before
        // this handle lists anything. Every candidate passes the same validated
        // lease and tombstone boundary as an explicit purge; corrupt, mismatched
        // and busy entries stay complete, and no clock means no expiry.
        library.purge_expired();
        Ok(library)
    }
    /// Automatic retention. Candidate identities are read first, then each one
    /// passes the same validated lease and tombstone boundary as an explicit
    /// purge; corrupt, mismatched and busy entries stay complete, and no clock
    /// means no expiry.
    fn purge_expired(&mut self) {
        let Some(now_unix_ms) = now_ms() else {
            return;
        };
        let Ok(retention) =
            RETENTION_DAYS
                .checked_mul(DAY_MS)
                .ok_or(Error::Edit(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                )))
        else {
            return;
        };
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        let mut expired = Vec::new();
        for (count, entry) in entries.enumerate() {
            if count >= MAX_ENTRIES * 4 {
                return;
            }
            let Ok(entry) = entry else {
                return;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str().and_then(|name| name.strip_prefix(TRASH)) else {
                continue;
            };
            if name.len() != 16
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                continue;
            }
            let Some(trashed_unix_ms) = read_trash_record(&entry.path()) else {
                continue;
            };
            if now_unix_ms.saturating_sub(trashed_unix_ms) < retention {
                continue;
            }
            if let Ok(id) = u64::from_str_radix(name, 16).map(ProjectId) {
                expired.push(id);
            }
        }
        for id in expired {
            // Busy, corrupt or mismatched entries refuse the lease and survive.
            let _ = self.purge_entry(id);
        }
    }
    /// The one permanent removal: admit the validated Trash entry, move it to an
    /// unlisted tombstone, then clean up. A failed cleanup leaves a `.delete-*`
    /// directory that is never listed, promoted or restored.
    fn purge_entry(&mut self, id: ProjectId) -> Result<()> {
        let lease = self.trash_lease(id)?;
        let tomb = self.unique("delete")?;
        lease.move_directory(&tomb)?;
        self.clear_history();
        let _ = fs::remove_dir_all(tomb);
        Ok(())
    }
    /// Move one active project to Trash. A missing wall clock refuses before the
    /// project is touched rather than recording an authoritative epoch.
    fn move_to_trash(&mut self, target: ProjectId, now_unix_ms: Option<u64>) -> Result<()> {
        let now_unix_ms =
            now_unix_ms.ok_or(Error::Edit(crate::EditError::Unsupported("system clock")))?;
        let lease = self.lease(target)?;
        write_trash_record(
            &self.directory(target),
            &TrashRecord {
                unix_ms: now_unix_ms,
            },
        )?;
        lease.move_directory(&self.trash_directory(target))?;
        self.clear_history();
        Ok(())
    }
    fn directory(&self, id: ProjectId) -> PathBuf {
        self.root.join(format!("{:016x}", id.0))
    }
    fn trash_directory(&self, id: ProjectId) -> PathBuf {
        self.root.join(format!("{TRASH}{:016x}", id.0))
    }
    fn lease(&self, id: ProjectId) -> Result<WriterLease> {
        self.directory_lease(&self.directory(id), id)
    }
    fn trash_lease(&self, id: ProjectId) -> Result<WriterLease> {
        self.directory_lease(&self.trash_directory(id), id)
    }
    fn directory_lease(&self, directory: &Path, id: ProjectId) -> Result<WriterLease> {
        if !fs::symlink_metadata(directory)?.file_type().is_dir()
            || !fs::symlink_metadata(directory.join(PROJECT))?
                .file_type()
                .is_file()
        {
            return Err(Error::Edit(crate::EditError::Invalid("library entry")));
        }
        let lease = WriterLease::acquire(&directory.join(PROJECT))?;
        if lease.load()?.project.data().id != id {
            return Err(Error::Edit(crate::EditError::Invalid("library identity")));
        }
        Ok(lease)
    }
    /// At most 1024 managed entries; corrupt entries return typed errors rather than
    /// silently hiding user projects. Busy projects may be listed read-only.
    pub fn list(&self) -> Result<Vec<ProjectEntry>> {
        self._lease.validate()?;
        let mut entries = Vec::new();
        let mut retained_bytes = 0usize;
        for (count, entry) in fs::read_dir(&self.root)?.enumerate() {
            if count >= MAX_ENTRIES * 4 {
                return Err(Error::Edit(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                )));
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.len() != 16
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                continue;
            }
            let (entry, bytes) = self.read_entry(&entry.path(), name, None)?;
            retained_bytes =
                retained_bytes
                    .checked_add(bytes)
                    .ok_or(Error::Edit(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    )))?;
            if retained_bytes > 16 << 20 || entries.len() >= MAX_ENTRIES {
                return Err(Error::Edit(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                )));
            }
            entries.push(entry);
        }
        entries.sort_by_key(|entry| entry.id.0);
        Ok(entries)
    }
    /// Trashed entries derive from their own complete directory and the store's
    /// move record; the project ID orders the listing for a stable view.
    pub fn list_trash(&self) -> Result<Vec<ProjectEntry>> {
        self._lease.validate()?;
        let mut entries = Vec::new();
        let mut retained_bytes = 0usize;
        for (count, entry) in fs::read_dir(&self.root)?.enumerate() {
            if count >= MAX_ENTRIES * 4 {
                return Err(Error::Edit(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                )));
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(name) = name.strip_prefix(TRASH) else {
                continue;
            };
            if name.len() != 16
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                continue;
            }
            let trashed_unix_ms = read_trash_record(&entry.path());
            let (entry, bytes) = self.read_entry(&entry.path(), name, trashed_unix_ms)?;
            retained_bytes =
                retained_bytes
                    .checked_add(bytes)
                    .ok_or(Error::Edit(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    )))?;
            if retained_bytes > 16 << 20 || entries.len() >= MAX_ENTRIES {
                return Err(Error::Edit(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                )));
            }
            entries.push(entry);
        }
        entries.sort_by_key(|entry| entry.id.0);
        Ok(entries)
    }
    fn read_entry(
        &self,
        directory: &Path,
        name: &str,
        trashed_unix_ms: Option<u64>,
    ) -> Result<(ProjectEntry, usize)> {
        if !fs::symlink_metadata(directory)?.file_type().is_dir() {
            return Err(Error::Edit(crate::EditError::Invalid("library entry")));
        }
        let id = ProjectId(u64::from_str_radix(name, 16).map_err(|_| {
            Error::Edit(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ))
        })?);
        let path = directory.join(PROJECT);
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(Error::Edit(crate::EditError::Invalid("library project")));
        }
        let project = project::load(&path)?.project;
        if project.data().id != id {
            return Err(Error::Edit(crate::EditError::Invalid("library identity")));
        }
        let metadata = project::read_metadata(&path);
        let thumbnail_bytes = match &metadata.thumbnail {
            Thumbnail::Ready { rgba, .. } => rgba.len(),
            _ => 0,
        };
        let bytes = project.data().name.as_str().len() + thumbnail_bytes + 512;
        Ok((
            ProjectEntry {
                id,
                short_id: format!("{:08x}", id.0 as u32),
                name: project.data().name.clone(),
                duration: project.data().scenes.main().duration(),
                created_unix_ms: metadata.created_unix_ms,
                modified_unix_ms: metadata.modified_unix_ms,
                thumbnail: metadata.thumbnail,
                path,
                trashed_unix_ms,
            },
            bytes,
        ))
    }
    /// Dispatch a registered command against one managed project. `project.create`
    /// uses its own ID; other commands target `target`. Undo/redo are library-local.
    pub fn execute(&mut self, target: ProjectId, command: Command) -> Result<Outcome> {
        self.execute_cancellable(target, command, &|| false)
    }
    /// Blocking execution; adapters run off the UI thread. Cancellation is checked
    /// between media chunks and before publication; a cancelled copy leaves no entry.
    pub fn execute_cancellable(
        &mut self,
        target: ProjectId,
        command: Command,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Outcome> {
        self._lease.validate()?;
        checkpoint(cancelled)?;
        match command {
            Command::CreateProject { id, name } => {
                self.check_name(&name, None)?;
                let project = Project::empty(id, name)?;
                self.publish(&project, id, Metadata::default(), cancelled)?;
                self.clear_history();
            }
            Command::DuplicateProject { id } => {
                let lease = self.lease(target)?;
                let source = lease.load()?.project;
                let hidden = self.unique("undo")?;
                let mut metadata = lease.metadata();
                let fresh = Metadata::default();
                metadata.created_unix_ms = fresh.created_unix_ms;
                metadata.modified_unix_ms = fresh.modified_unix_ms;
                let hash = self.publish(&source, id, metadata, cancelled)?;
                self.record(Step::Duplicate { id, hash, hidden });
            }
            Command::RenameProject { name } => {
                self.check_name(&name, Some(target))?;
                let mut lease = self.lease(target)?;
                let before = lease.load()?.project;
                let mut session = Session::new(before.clone(), 1)?;
                session.apply(Command::RenameProject { name })?;
                let after = session.project().as_ref().clone();
                if before == after {
                    return Ok(outcome(false));
                }
                let before_hash = fingerprint(&before)?;
                let after_hash = fingerprint(&after)?;
                lease.save(&after)?;
                let mut metadata = lease.metadata();
                metadata.touch();
                let _ = lease.save_metadata(&metadata);
                self.record(Step::Rename {
                    id: target,
                    before: before.data().name.clone(),
                    after: after.data().name.clone(),
                    before_hash,
                    after_hash,
                });
            }
            Command::DeleteProject {} => {
                // Confirmation belongs to the caller. Trash keeps the complete
                // directory, so a restore never relinks media.
                self.move_to_trash(target, now_ms())?;
            }
            Command::RestoreProject {} => {
                let lease = self.trash_lease(target)?;
                let destination = self.directory(target);
                lease.move_directory(&destination)?;
                self.clear_history();
                // The record describes a Trash entry; a failed removal leaves an
                // ignored file that the next deletion replaces.
                let _ = fs::remove_file(destination.join(TRASH_RECORD));
            }
            Command::PurgeProject {} => {
                // Confirmation belongs to the caller, as for any permanent delete.
                self.purge_entry(target)?;
            }
            Command::ThumbnailProject {} => {
                let lease = self.lease(target)?;
                let project = lease.load()?.project;
                let mut metadata = lease.metadata();
                metadata.thumbnail = crate::store::project_cover::capture(
                    &project,
                    &crate::ByteBudgetHooks::new(&self.budget, None),
                );
                lease.save_metadata(&metadata)?;
            }
            Command::Undo {} => self.undo_cancellable(cancelled)?,
            Command::Redo {} => self.redo_cancellable(cancelled)?,
            _ => {
                return Err(Error::Edit(crate::EditError::Invalid(
                    "command requires a project session",
                )));
            }
        }
        Ok(outcome(true))
    }
    fn check_name(&self, name: &NfcText, except: Option<ProjectId>) -> Result<()> {
        if name.as_str().trim().is_empty() {
            return Err(Error::Edit(crate::EditError::Invalid("empty project name")));
        }
        if self
            .list()?
            .iter()
            .any(|entry| Some(entry.id) != except && entry.name == *name)
        {
            return Err(Error::Edit(crate::EditError::Invalid(
                "project name already exists",
            )));
        }
        Ok(())
    }
    fn unique(&self, prefix: &str) -> Result<PathBuf> {
        let mut bytes = [0u8; 16];
        crate::session::identity_entropy(&mut bytes)?;
        Ok(self
            .root
            .join(format!(".{prefix}-{}", blake3::hash(&bytes).to_hex())))
    }
    fn publish(
        &self,
        source: &Project,
        id: ProjectId,
        mut metadata: Metadata,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<[u8; 32]> {
        if self.list()?.len() >= MAX_ENTRIES {
            return Err(Error::Edit(EditError::Control(
                crate::session::wire::ControlError::Limit,
            )));
        }
        let destination = self.directory(id);
        if destination.try_exists()? {
            return Err(Error::Edit(crate::EditError::Invalid(
                "project already exists",
            )));
        }
        let mut data = source.data().clone();
        data.id = id;
        // Duplicates retain the exact project name/content. Rename separately.
        let stage = self.unique("stage")?;
        private_directory(&stage, false)?;
        let mut guard = DirectoryGuard(Some(stage.clone()));
        private_directory(&stage.join("media"), false)?;
        let mut total = 0u64;
        for asset in &mut data.assets {
            checkpoint(cancelled)?;
            if !fs::metadata(asset.path())?.is_file() {
                return Err(Error::Edit(crate::EditError::Invalid("media source")));
            }
            let mut file = fs::File::open(asset.path())?;
            if !file.metadata()?.is_file() {
                return Err(Error::Edit(crate::EditError::Invalid("media source")));
            }
            let relative = PathBuf::from("media").join(format!("{:016x}", asset.id().0));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options.open(stage.join(&relative))?;
            let mut hasher = blake3::Hasher::new();
            let mut buffer = [0u8; 65536];
            loop {
                checkpoint(cancelled)?;
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                total = total
                    .checked_add(n as u64)
                    .ok_or(Error::Edit(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    )))?;
                if total > 1 << 40 {
                    return Err(Error::Edit(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    )));
                }
                let bytes = buffer.get(..n).ok_or(Error::Edit(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                )))?;
                output.write_all(bytes)?;
                hasher.update(bytes);
            }
            output.sync_all()?;
            if *hasher.finalize().as_bytes() != asset.content_hash().0 {
                return Err(Error::Edit(crate::EditError::Invalid(
                    "media content changed",
                )));
            }
            let mut value: media::AssetData = asset.clone().into();
            value.path = destination.join(relative);
            *asset = value.try_into()?;
        }
        if !data.luts.is_empty() {
            private_directory(&stage.join("media/luts"), false)?;
            for record in &mut data.luts {
                checkpoint(cancelled)?;
                let relative = PathBuf::from("media/luts").join(record.id.hex());
                super::lut_storage::copy_bundle(
                    record,
                    &stage.join(&relative),
                    &self.budget,
                    cancelled,
                )?;
                record.directory = destination.join(relative);
            }
        }
        let project = Project::new(data)?;
        let hash = fingerprint(&project)?;
        // Capture from the source while rewritten destination paths are not published.
        metadata.thumbnail = crate::store::project_cover::capture(
            source,
            &crate::ByteBudgetHooks::new(&self.budget, None),
        );
        let mut writer = WriterLease::acquire(&stage.join(PROJECT))?;
        writer.save(&project)?;
        writer.save_metadata(&metadata)?;
        drop(writer);
        // Source/destination remain on one filesystem. The catalog lease serializes
        // managed publishers, and a failed copy never creates a visible entry.
        if destination.try_exists()? {
            return Err(Error::Edit(crate::EditError::Invalid(
                "project already exists",
            )));
        }
        checkpoint(cancelled)?;
        self._lease.validate()?;
        fs::rename(&stage, &destination)?;
        guard.0 = None;
        Ok(hash)
    }
    fn record(&mut self, step: Step) {
        for old in self.redo.drain(..) {
            cleanup(old);
        }
        self.undo.push_back(step);
        if self.undo.len() > HISTORY
            && let Some(old) = self.undo.pop_front()
        {
            cleanup(old);
        }
    }
    fn clear_history(&mut self) {
        for step in self.undo.drain(..).chain(self.redo.drain(..)) {
            cleanup(step);
        }
    }
    fn apply_history(&self, step: &Step, redo: bool, cancelled: &dyn Fn() -> bool) -> Result<()> {
        checkpoint(cancelled)?;
        self._lease.validate()?;
        match step {
            Step::Rename {
                id,
                before,
                after,
                before_hash,
                after_hash,
            } => {
                let (expected, replacement) = if redo {
                    (before_hash, after)
                } else {
                    (after_hash, before)
                };
                let mut lease = self.lease(*id)?;
                let current = lease.load()?.project;
                if fingerprint(&current)? != *expected {
                    return Err(project::Error::Conflict.into());
                }
                let mut data = current.into_data();
                data.name = replacement.clone();
                lease.save(&Project::new(data)?)?;
                let mut metadata = lease.metadata();
                metadata.touch();
                let _ = lease.save_metadata(&metadata);
            }
            Step::Duplicate { id, hash, hidden } => {
                let destination = self.directory(*id);
                if redo {
                    if destination.try_exists()? {
                        return Err(project::Error::Conflict.into());
                    }
                    if !fs::symlink_metadata(hidden)?.file_type().is_dir()
                        || !fs::symlink_metadata(hidden.join(PROJECT))?
                            .file_type()
                            .is_file()
                    {
                        return Err(Error::Edit(crate::EditError::Invalid(
                            "library history entry",
                        )));
                    }
                    let lease = WriterLease::acquire(&hidden.join(PROJECT))?;
                    if fingerprint(&lease.load()?.project)? != *hash {
                        return Err(project::Error::Conflict.into());
                    }
                    lease.move_directory(&destination)?;
                } else {
                    let lease = self.lease(*id)?;
                    let project = lease.load()?.project;
                    if fingerprint(&project)? != *hash {
                        return Err(project::Error::Conflict.into());
                    }
                    for asset in &project.data().assets {
                        if media::ContentHash::read_with_cancel(asset.path(), cancelled, |_, _| {})?
                            != asset.content_hash()
                        {
                            return Err(project::Error::Conflict.into());
                        }
                    }
                    for record in &project.data().luts {
                        let _lut_memory = self
                            .budget
                            .reserve(crate::ByteOwner::PersistentCache, 256 << 10)?;
                        super::lut_storage::verify_bundle(record, cancelled)?;
                    }
                    checkpoint(cancelled)?;
                    lease.move_directory(hidden)?;
                }
            }
        }
        Ok(())
    }
    /// Available catalog undo/redo steps, independent of Session content history.
    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn undo(&mut self) -> Result<()> {
        self.undo_cancellable(&|| false)
    }
    fn undo_cancellable(&mut self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let step = self.undo.back().ok_or(Error::Edit(EditError::Session(
            crate::session::SessionError::NoHistory,
        )))?;
        self.apply_history(step, false, cancelled)?;
        if let Some(step) = self.undo.pop_back() {
            self.redo.push(step);
        }
        Ok(())
    }
    pub fn redo(&mut self) -> Result<()> {
        self.redo_cancellable(&|| false)
    }
    fn redo_cancellable(&mut self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let step = self.redo.last().ok_or(Error::Edit(EditError::Session(
            crate::session::SessionError::NoHistory,
        )))?;
        self.apply_history(step, true, cancelled)?;
        if let Some(step) = self.redo.pop() {
            self.undo.push_back(step);
        }
        Ok(())
    }
}
fn outcome(changed: bool) -> Outcome {
    Outcome {
        changed,
        created: Vec::new(),
        placements: Vec::new(),
    }
}
fn cleanup(step: Step) {
    if let Step::Duplicate { hidden, .. } = step {
        let _ = fs::remove_dir_all(hidden);
    }
}
impl Drop for ProjectLibrary {
    fn drop(&mut self) {
        self.clear_history();
    }
}
pub(super) struct DirectoryGuard(pub(super) Option<PathBuf>);
pub(super) fn staging_directory(parent: &Path, prefix: &str) -> crate::runtime::Result<PathBuf> {
    let mut bytes = [0; 16];
    crate::session::identity_entropy(&mut bytes)
        .map_err(|error| RuntimeError::Edit(error.into()))?;
    let path = parent.join(format!(".{prefix}-{}", blake3::hash(&bytes).to_hex()));
    private_directory(&path, false)?;
    Ok(path)
}
impl Drop for DirectoryGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn checkpoint(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(Error::Runtime(crate::RuntimeError::Cancelled))
    } else {
        Ok(())
    }
}

/// A representable wall clock, or `None` when the host cannot supply one. The
/// destructive retention policy treats `None` as "no expiry", never as epoch.
fn now_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
}

/// Write the move record before the directory moves, so a failed move leaves the
/// entry active. Publication replaces the directory entry through the atomic
/// record owner: an existing hard link to project or media bytes is never edited
/// in place, and symlink or writer-namespace destinations refuse.
fn write_trash_record(directory: &Path, record: &TrashRecord) -> Result<()> {
    let path = directory.join(TRASH_RECORD);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(Error::Edit(crate::EditError::Invalid(
                "library trash record",
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let bytes = serde_json::to_vec(record)
        .map_err(|_| Error::Edit(crate::EditError::Invalid("library trash record")))?;
    if bytes.len() > 128 {
        return Err(Error::Edit(EditError::Control(
            crate::session::wire::ControlError::Limit,
        )));
    }
    project::atomic_write(&path, &bytes)?;
    Ok(())
}

/// A missing, unreadable or corrupt record keeps the entry listed but untimed,
/// so nothing is ever purged without an authoritative move time.
fn read_trash_record(directory: &Path) -> Option<u64> {
    let path = directory.join(TRASH_RECORD);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 128 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let record: TrashRecord = serde_json::from_slice(&bytes).ok()?;
    Some(record.unix_ms)
}

fn private_directory(path: &Path, recursive: bool) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn fingerprint(project: &Project) -> Result<[u8; 32]> {
    Ok(*blake3::hash(&project::encode(project, false)?).as_bytes())
}
impl std::fmt::Debug for ProjectEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectEntry")
            .field("id", &self.id)
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_clock_refuses_a_trash_move_and_keeps_the_project_active() {
        let root = tempfile::tempdir().expect("directory");
        let mut library =
            ProjectLibrary::open(root.path(), ByteBudget::new(16 << 20).expect("budget"))
                .expect("library");
        library
            .execute(
                ProjectId(1),
                Command::CreateProject {
                    id: ProjectId(1),
                    name: "Clock".into(),
                },
            )
            .expect("create");
        assert!(matches!(
            library.move_to_trash(ProjectId(1), None),
            Err(Error::Edit(crate::EditError::Unsupported("system clock")))
        ));
        assert_eq!(library.list().expect("list").len(), 1);
        assert!(library.list_trash().expect("trash").is_empty());
        assert!(!library.directory(ProjectId(1)).join(TRASH_RECORD).exists());
        // A representable clock still moves the same project.
        library
            .move_to_trash(ProjectId(1), Some(1_800_000_000_000))
            .expect("trash");
        assert_eq!(library.list_trash().expect("trash").len(), 1);
    }
}

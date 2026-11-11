//! Publishes scheduled recoverable checkpoints.
use super::{
    Arc, ArtifactKind, Autosave, CacheKey, EditError, Error, Flight, JOURNAL, JOURNAL_VERSION,
    JobBytes, JobKind, JobSpec, JobState, Journal, KeyInputs, MAX_META, MAX_PROJECT, Mutex, Path,
    PathBuf, Project, PumpFailure, ResourceClass, Result, SaveFlight, SaveSlot, SaveWork,
    SystemTime, UNIX_EPOCH, WORK_BYTES, WorkingSave, WriterLease, clear, fs, read_bounded,
};
fn publish(
    directory: &Path,
    project: &Project,
    saved_path: Option<PathBuf>,
    revision: u64,
) -> Result<()> {
    // Stop permanently on any failure. With two snapshots, even failure between
    // snapshot and descriptor publication preserves the preceding committed pair.
    let path = project::autosave::save(&directory.join("project"), project, 2)?;
    let bytes = read_bounded(&path, MAX_PROJECT)?;
    let journal = Journal {
        version: JOURNAL_VERSION,
        project_id: project.data().id,
        name: project.data().name.as_str().to_owned(),
        saved_path,
        revision,
        content_hash: *blake3::hash(&bytes).as_bytes(),
        timestamp_unix_ms: u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Error::Invalid("system clock"))?
                .as_millis(),
        )
        .map_err(|_| Error::Limit)?,
        snapshot: path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(Error::Corrupt)?
            .to_owned(),
    };
    let bytes =
        serde_json::to_vec(&journal).map_err(|_| Error::Unsupported("journal path encoding"))?;
    if bytes.len() as u64 > MAX_META {
        return Err(Error::Limit);
    }
    project::atomic_write(&directory.join(JOURNAL), &bytes)?;
    Ok(())
}
impl Drop for WorkingSave {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = self.work.take();
        }
    }
}
fn save_on_worker(
    slot: &SaveSlot,
    project: &Project,
    metadata: &project::Metadata,
) -> crate::Result<()> {
    let work = slot
        .lock()
        .map_err(|_| EditError::from(crate::RuntimeError::Poisoned))?
        .take()
        .ok_or(Error::Busy)?;
    let mut active = WorkingSave {
        slot: slot.clone(),
        work: Some(work),
    };
    let work = active.work.as_mut().ok_or(Error::Busy)?;
    work.receipt = Some(if let Some(source) = &work.source {
        source.save_to(&mut work.writer, project)?
    } else {
        work.writer.save(project)?
    });
    work.writer.save_metadata(metadata)?;
    work.metadata = Some(metadata.clone());
    Ok(())
}
impl From<EditError> for PumpFailure {
    fn from(error: EditError) -> Self {
        Self::Persistence(error)
    }
}

impl Autosave {
    pub(super) fn observe(&mut self) -> crate::Result<()> {
        if self.session.transaction_active() {
            return Ok(());
        }
        let now = self.clock.now();
        if !Arc::ptr_eq(&self.observed, self.session.project()) {
            self.revision = self.revision.checked_add(1).ok_or(EditError::Limit)?;
            self.observed = self.session.project().clone();
            self.last_change = now;
        }
        if self.is_dirty()
            && self.revision != self.journaled
            && self
                .flight
                .as_ref()
                .is_none_or(|f| f.revision != self.revision || f.clear)
        {
            self.first_dirty.get_or_insert(now);
        }
        Ok(())
    }
}

impl Autosave {
    /// Nonblocking event-loop pump. A failed job is sticky: no later publication
    /// can rotate away the last confirmed snapshot. Errors must be presented.
    pub fn tick(&mut self) -> crate::Result<()> {
        if let Some(e) = &self.failure {
            return Err(Error::PersistenceFailed(e.clone()).into());
        }
        match self.pump() {
            Ok(()) => Ok(()),
            Err(PumpFailure::Admission(error)) => Err(error),
            Err(PumpFailure::Persistence(EditError::StaleResult)) => Err(EditError::StaleResult),
            Err(PumpFailure::Persistence(error)) => {
                let error: Arc<dyn crate::Failure> = Arc::new(error);
                self.failure = Some(error.clone());
                Err(Error::PersistenceFailed(error).into())
            }
        }
    }
}

impl Autosave {
    pub(super) fn pump(&mut self) -> std::result::Result<(), PumpFailure> {
        self.observe()?;
        self.runner.tick()?;
        self.runner.drain_events();
        if let Some(f) = &self.flight {
            let state = self
                .runner
                .snapshot(f.id)
                .ok_or(EditError::WorkerPanicked)?;
            match state.state {
                JobState::Failed | JobState::Cancelled => {
                    let cause = state
                        .error
                        .unwrap_or_else(|| Arc::new(EditError::WorkerPanicked));
                    return Err(EditError::from(Error::PersistenceFailed(cause)).into());
                }
                JobState::Completed => {
                    let f = self.flight.take().ok_or(EditError::WorkerPanicked)?;
                    self.runner
                        .take_output(f.id)
                        .ok_or(EditError::WorkerPanicked)?;
                    self.journaled = if f.clear { 0 } else { f.revision };
                    if self.revision == f.revision {
                        self.first_dirty = None;
                    }
                    if let Some(save) = f.save {
                        let mut work = save
                            .slot
                            .lock()
                            .map_err(|_| EditError::from(crate::RuntimeError::Poisoned))?
                            .take()
                            .ok_or(EditError::WorkerPanicked)?;
                        let receipt = work.receipt.take().ok_or(EditError::WorkerPanicked)?;
                        let metadata = work.metadata.take().ok_or(EditError::WorkerPanicked)?;
                        if receipt.path() != work.writer.path()
                            || receipt.project_id() != f.snapshot.data().id
                        {
                            return Err(EditError::StaleResult.into());
                        }
                        // An in-place save advances the held baseline even if the
                        // edited revision is now stale. Save-as retains its source
                        // lease until this revision can adopt the new destination.
                        let current = self.session.matches_target(&f.target)
                            && !self.session.transaction_active();
                        if save.in_place || current {
                            self.writer = Some(work.writer);
                        } else {
                            self.writer = work.source;
                        }
                        // Revision identity includes gesture rollback/save boundaries,
                        // not just equal content or a matching numeric revision.
                        if current {
                            self.session.restore_metadata(metadata)?;
                            self.session.saved()?;
                            if !save.in_place {
                                self.session.adopt_saved_project(f.snapshot.clone());
                                self.observed = f.snapshot.clone();
                            }
                            self.saved_path = Some(receipt.path().to_owned());
                            self.saved_receipt = Some(receipt);
                            self.saved = Some(f.snapshot);
                        } else {
                            // The destination was written even though this revision
                            // cannot be acknowledged. Undo may restore the old saved
                            // Arc, which no longer proves equality with disk. Treat
                            // all stale destinations conservatively (including path
                            // aliases); only a fresh save may reestablish cleanliness.
                            self.saved = None;
                            self.saved_receipt = None;
                            return Err(EditError::StaleResult.into());
                        }
                    }
                }
                _ => {}
            }
        }
        if self.flight.is_none()
            && self.pending_save.is_some()
            && !self.session.transaction_active()
        {
            let path = self
                .pending_save
                .take()
                .ok_or_else(|| EditError::from(Error::Busy))?;
            if let Err(error) = self.start(Some(path)) {
                return Err(if self.flight.is_none() {
                    PumpFailure::Admission(error)
                } else {
                    PumpFailure::Persistence(error)
                });
            }
        }
        if self.flight.is_none()
            && !self.session.transaction_active()
            && !self.is_dirty()
            && self.journaled != 0
        {
            self.start_task(None, true)?;
        }
        if self.flight.is_none()
            && !self.session.transaction_active()
            && self.is_dirty()
            && self.revision != self.journaled
        {
            let now = self.clock.now();
            if self
                .first_dirty
                .is_some_and(|first| now.saturating_sub(first) >= self.schedule.max_dirty_age)
                || now.saturating_sub(self.last_change) >= self.schedule.debounce
            {
                self.start(None)?;
            }
        }
        Ok(())
    }
}

impl Autosave {
    pub(super) fn start(&mut self, save_path: Option<PathBuf>) -> crate::Result<()> {
        self.start_task(save_path, false)
    }
}

impl Autosave {
    pub(super) fn start_task(
        &mut self,
        save_path: Option<PathBuf>,
        clear_journal: bool,
    ) -> crate::Result<()> {
        if self.flight.is_some() {
            return Err(Error::Busy.into());
        }
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        let snapshot = self.session.project().clone();
        let mut metadata = self.session.metadata().clone();
        metadata.touch();
        let mut project = snapshot.clone();
        let directory = self.directory.clone();
        let old_path = self.saved_path.clone();

        let revision = self.revision;
        let input = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "engine",
            model: "session-journal",
            version: "1",
            settings: &revision.to_le_bytes(),
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        #[cfg(test)]
        let fault = self.fault;
        let save = if let Some(path) = save_path {
            // Only explicit save-as acquires a new destination. Existing project
            // saves reuse the lease acquired before load; aliases follow its path.
            let in_place = self.writer.as_ref().is_some_and(|writer| {
                fs::canonicalize(&path).is_ok_and(|path| path == writer.path())
                    || path == writer.path()
            });
            if !in_place {
                project = Arc::new(snapshot.fork()?);
            }
            let writer = if in_place {
                self.writer.take().ok_or(Error::Busy)?
            } else {
                WriterLease::acquire(&path)?
            };
            Some(SaveFlight {
                destination: writer.path().to_owned(),
                slot: Arc::new(Mutex::new(Some(SaveWork {
                    source: if in_place { None } else { self.writer.take() },
                    writer,
                    receipt: None,
                    metadata: None,
                }))),
                in_place,
            })
        } else {
            None
        };
        let published_snapshot = project.clone();
        let slot = save.as_ref().map(|save| save.slot.clone());
        let recovery_snapshot = snapshot.clone();
        let submitted = self.runner.submit(
            JobSpec {
                kind: JobKind::Persist,
                class: ResourceClass::Cpu,
                input,
                bytes: JobBytes {
                    compositor: WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                #[cfg(test)]
                if let Some(fault) = fault {
                    fault()?;
                }
                if clear_journal {
                    clear(&directory)?;
                } else {
                    publish(&directory, &recovery_snapshot, old_path.clone(), revision)?;
                }
                if let Some(slot) = &slot {
                    let mut metadata = metadata.clone();
                    metadata.thumbnail = crate::store::project_cover::capture(&project, ctx);
                    save_on_worker(slot, &project, &metadata)?;
                }
                Ok(Vec::new())
            },
        );
        let (id, _) = match submitted {
            Ok(submitted) => submitted,
            Err(error) => {
                if let Some(save) = save {
                    let work = save
                        .slot
                        .lock()
                        .map_err(|_| EditError::from(crate::RuntimeError::Poisoned))?
                        .take()
                        .ok_or(EditError::WorkerPanicked)?;
                    self.writer = if save.in_place {
                        Some(work.writer)
                    } else {
                        work.source
                    };
                }
                return Err(error);
            }
        };
        self.flight = Some(Flight {
            id,
            target: self.session.job_target(),
            revision,
            snapshot: published_snapshot,
            save,
            clear: clear_journal,
        });
        self.first_dirty = None;
        self.runner.tick()?;
        Ok(())
    }
}

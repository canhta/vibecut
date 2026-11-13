//! Executes persistent session lifecycle operations.
use super::{Autosave, Duration, Error, PathBuf, Result, Session, clear};
use crate::Lifecycle;
impl Autosave {
    /// Reserve the destination, then queue journal/project publication off-thread.
    /// Native path resolution/lease acquisition can block this caller. The journal
    /// precedes the project write. Poll tick until saving() is false; leaving before
    /// acknowledgement is not a successful save boundary.
    pub fn save_to(&mut self, path: PathBuf) -> Result<()> {
        if !path.is_absolute() {
            return Err(Error::Edit(crate::session::EditError::Invalid(
                "save path must be absolute",
            )));
        }
        // WriterLease resolves existing symlinks before publication. Apply the
        // storage boundary to that destination, not merely the supplied parent.
        let destination = match path.canonicalize() {
            Ok(destination) => destination,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.clone(),
            Err(error) => return Err(error.into()),
        };
        let parent = destination
            .parent()
            .ok_or(Error::Edit(crate::session::EditError::Invalid(
                "save destination",
            )))?
            .canonicalize()?;
        let root = self
            .directory
            .parent()
            .ok_or(Error::Corrupt)?
            .canonicalize()?;
        if parent.starts_with(root) {
            return Err(Error::Edit(crate::session::EditError::Invalid(
                "save destination is inside recovery storage",
            )));
        }
        self.tick()?;
        if self.saving() {
            return Err(Error::Busy);
        }
        if self.session.transaction_active() {
            return Err(Error::from(crate::session::SessionError::TransactionActive));
        }
        self.pending_save = Some(path);
        self.tick()
    }
}

impl Autosave {
    /// Typed lifecycle dispatch: the declared route picks the persistence owner, and
    /// completion of a save is acknowledged by tick().
    pub fn apply(&mut self, command: crate::Command) -> Result<crate::Outcome> {
        match command.lifecycle() {
            Some(Lifecycle::Thumbnail) => {
                self.refresh_thumbnail()?;
                Ok(unchanged())
            }
            Some(Lifecycle::Save) => {
                self.save()?;
                Ok(unchanged())
            }
            Some(
                Lifecycle::Create
                | Lifecycle::Duplicate
                | Lifecycle::Delete
                | Lifecycle::Restore
                | Lifecycle::Purge
                | Lifecycle::SaveAs
                | Lifecycle::ExportBundle
                | Lifecycle::ImportBundle,
            )
            | None => Ok(self.session.apply(command)?),
        }
    }
    fn refresh_thumbnail(&mut self) -> Result<()> {
        if self.session.transaction_active() {
            return Err(Error::from(crate::session::SessionError::TransactionActive));
        }
        self.flush()?;
        let mut metadata = self.session.metadata().clone();
        metadata.thumbnail = crate::store::project_cover::capture(
            self.session.project(),
            &crate::ByteBudgetHooks::new(&self.budget, None),
        );
        if let Some(writer) = &self.writer {
            writer
                .save_metadata(&metadata)
                .map_err(crate::session::EditError::from)?;
        }
        Ok(self.session.restore_metadata(metadata)?)
    }
}
fn unchanged() -> crate::Outcome {
    crate::Outcome {
        changed: false,
        created: Vec::new(),
        placements: Vec::new(),
    }
}

impl Autosave {
    pub fn save(&mut self) -> Result<()> {
        let path = self.saved_path.clone().ok_or(Error::SaveAsRequired)?;
        self.save_to(path)
    }
}

impl Autosave {
    /// Close this session after its work was published elsewhere (a Save As copy
    /// the window has reopened). Settles any in-flight job so it cannot rewrite the
    /// journal, then removes the journal and snapshots so the abandoned session is
    /// not offered for recovery. The project file this session may have been saved
    /// to is never touched.
    pub fn retire(mut self) -> Result<()> {
        self.flush()?;
        clear(&self.directory)
    }
}

impl Autosave {
    /// Acknowledge the old pending revision before replacing the active project.
    /// On failure this retains the current Session and refuses the switch.
    pub fn switch(&mut self, session: Session) -> Result<()> {
        self.flush()?;
        let root = self.directory.parent().ok_or(Error::Corrupt)?;
        let next = Self::new(
            root,
            session,
            self.budget.clone(),
            self.clock.clone(),
            self.schedule,
        )?;
        *self = next;
        Ok(())
    }
}

impl Autosave {
    /// Explicit blocking project-switch/exit barrier. IO remains on the job worker;
    /// the app should keep its progress presentation responsive while waiting.
    /// Failure refuses the boundary; Drop alone never claims to have flushed.
    pub fn flush(&mut self) -> Result<u64> {
        if self.session.transaction_active() {
            return Err(Error::from(crate::session::SessionError::TransactionActive));
        }
        self.tick()?;
        while self.flight.is_some() {
            std::thread::sleep(Duration::from_millis(1));
            self.tick()?;
        }
        if self.is_dirty() && self.journaled != self.revision {
            self.start(None)?;
            while self.flight.is_some() {
                std::thread::sleep(Duration::from_millis(1));
                self.tick()?;
            }
        }
        Ok(self.journaled)
    }
}

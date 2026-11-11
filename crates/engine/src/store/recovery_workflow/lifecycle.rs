//! Executes persistent session lifecycle operations.
use super::{Autosave, Duration, EditError, Error, PathBuf, Session};
use crate::Lifecycle;
impl Autosave {
    /// Reserve the destination, then queue journal/project publication off-thread.
    /// Native path resolution/lease acquisition can block this caller. The journal
    /// precedes the project write. Poll tick until saving() is false; leaving before
    /// acknowledgement is not a successful save boundary.
    pub fn save_as(&mut self, path: PathBuf) -> crate::Result<()> {
        if !path.is_absolute() {
            return Err(EditError::Invalid("save path must be absolute"));
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
            .ok_or(EditError::Invalid("save destination"))?
            .canonicalize()?;
        let root = self
            .directory
            .parent()
            .ok_or(Error::Corrupt)?
            .canonicalize()?;
        if parent.starts_with(root) {
            return Err(EditError::Invalid(
                "save destination is inside recovery storage",
            ));
        }
        self.tick()?;
        if self.saving() {
            return Err(Error::Busy.into());
        }
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.pending_save = Some(path);
        self.tick()
    }
}

impl Autosave {
    /// Typed lifecycle dispatch: the declared route picks the persistence owner, and
    /// completion of a save is acknowledged by tick().
    pub fn apply(&mut self, command: crate::Command) -> crate::Result<crate::Outcome> {
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
                | Lifecycle::Purge,
            )
            | None => self.session.apply(command),
        }
    }
    /// Dispatch a registry command; the declared lifecycle route picks the persistence
    /// owner, and completion of a save is acknowledged by tick().
    pub fn execute(
        &mut self,
        id: &str,
        params: serde_json::Value,
    ) -> crate::Result<crate::Outcome> {
        match crate::lifecycle(id) {
            Some(Lifecycle::Thumbnail) => {
                self.empty_params(&params)?;
                self.refresh_thumbnail()?;
                Ok(unchanged())
            }
            Some(Lifecycle::Save) => {
                self.empty_params(&params)?;
                self.save()?;
                Ok(unchanged())
            }
            Some(
                Lifecycle::Create
                | Lifecycle::Duplicate
                | Lifecycle::Delete
                | Lifecycle::Restore
                | Lifecycle::Purge,
            )
            | None => self.session.execute(id, params),
        }
    }
    fn empty_params(&self, params: &serde_json::Value) -> crate::Result<()> {
        if *params == serde_json::json!({}) {
            Ok(())
        } else {
            Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ))
        }
    }
    fn refresh_thumbnail(&mut self) -> crate::Result<()> {
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.flush()?;
        let mut metadata = self.session.metadata().clone();
        metadata.thumbnail = crate::store::project_cover::capture(
            self.session.project(),
            &crate::ByteBudgetHooks::new(&self.budget, None),
        );
        if let Some(writer) = &self.writer {
            writer.save_metadata(&metadata)?;
        }
        self.session.restore_metadata(metadata)
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
    pub fn save(&mut self) -> crate::Result<()> {
        let path = self.saved_path.clone().ok_or(Error::SaveAsRequired)?;
        self.save_as(path)
    }
}

impl Autosave {
    /// Acknowledge the old pending revision before replacing the active project.
    /// On failure this retains the current Session and refuses the switch.
    pub fn switch(&mut self, session: Session) -> crate::Result<()> {
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
    pub fn flush(&mut self) -> crate::Result<u64> {
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
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

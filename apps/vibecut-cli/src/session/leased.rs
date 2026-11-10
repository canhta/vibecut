//! Runs one one-shot edit over a leased, restored project session.
use crate::args::CliError;
use engine::Session;
use project::{SaveReceipt, WriterLease};
use std::path::Path;

/// Undo depth a one-shot session keeps.
#[derive(Clone, Copy)]
pub(crate) enum History {
    /// One snapshot: every one-shot command except `cut`.
    Single,
    /// The 100 snapshots `cut` kept for reviewing many pause spans at once.
    PauseReview,
}
impl History {
    fn limit(self) -> usize {
        match self {
            Self::Single => 1,
            Self::PauseReview => 100,
        }
    }
}

/// What the helper publishes after the closure.
#[derive(Clone, Copy)]
pub(crate) enum WritePolicy {
    /// Save only a changed project; touch metadata only for a changed command.
    Changed,
    /// Republish project and metadata unchanged, without touching metadata.
    Always,
}

/// A closure's value and whether its command reported a change.
pub(crate) struct Edit<T> {
    pub(crate) value: T,
    pub(crate) changed: bool,
}

/// A closure's value and the receipt of the project the helper saved.
pub(crate) struct Saved<T> {
    pub(crate) value: T,
    pub(crate) receipt: Option<SaveReceipt>,
}

/// Acquire the writer lease, load the project, restore sidecar metadata, run
/// `edit`, then publish once according to `write`.
///
/// [`WritePolicy::Changed`] is the rule for five one-shot commands: save when
/// the project bytes changed, call [`Session::saved`] after that save, and touch
/// and save metadata when the command reported a change. `transcript` did not
/// compare bytes before saving, but a successful word-range cut always changes
/// the project, so the unified rule keeps its observed output and bytes.
/// [`WritePolicy::Always`] is the observed rule for `exec project.save` and
/// `sounds add`: both republish project and metadata without touching metadata
/// or advancing the save boundary. `cut` keeps 100 undo snapshots; every other
/// command keeps 1.
pub(crate) fn run_leased<T>(
    path: &Path,
    history: History,
    write: WritePolicy,
    edit: impl FnOnce(&mut Session) -> Result<Edit<T>, CliError>,
) -> Result<Saved<T>, CliError> {
    let mut lease = WriterLease::acquire(path)?;
    let loaded = lease.load()?;
    let mut session = Session::new(loaded.project, history.limit())?;
    session.restore_metadata(lease.metadata())?;
    let before = session.project().clone();
    let edit = edit(&mut session)?;
    let receipt = match write {
        WritePolicy::Changed => {
            let receipt = if before != *session.project() {
                let receipt = lease.save(session.project().as_ref())?;
                session.saved()?;
                Some(receipt)
            } else {
                None
            };
            if edit.changed {
                let mut metadata = session.metadata().clone();
                metadata.touch();
                lease.save_metadata(&metadata)?;
            }
            receipt
        }
        WritePolicy::Always => {
            let receipt = lease.save(session.project())?;
            lease.save_metadata(session.metadata())?;
            Some(receipt)
        }
    };
    Ok(Saved {
        value: edit.value,
        receipt,
    })
}

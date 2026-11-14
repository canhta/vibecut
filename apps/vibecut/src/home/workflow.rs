//! Runs the managed catalog's requests on one worker thread.
//!
//! The worker owns the exclusive [`ProjectLibrary`] handle; every request is a
//! registered engine command, so no filesystem call blocks the UI thread.

use engine::{
    ByteBudget, Command,
    library::{ProjectEntry, ProjectLibrary, UnreadableProject},
};
use project::ProjectId;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Receiver,
    },
};

/// The managed library the catalog worker opened, lent to the other windows of the process.
/// The library holds one exclusive catalog lease, so no window opens a second handle: the
/// worker publishes its handle here after a successful open and withdraws it when it stops.
/// Calls block, so callers run them off the UI thread.
#[derive(Clone, Default)]
pub struct Catalog(Arc<Mutex<Option<Arc<Mutex<ProjectLibrary>>>>>);

impl Catalog {
    /// Run `run` against the library, or fail typed while the catalog is not open.
    pub fn with<T>(
        &self,
        run: impl FnOnce(&mut ProjectLibrary) -> engine::library::Result<T>,
    ) -> engine::control::Result<T> {
        // The handle is held only for this call, so a Retry that reopens the catalog is never
        // kept busy by a lent clone.
        let library = self
            .0
            .lock()
            .map_err(|_| engine::RuntimeError::Poisoned)?
            .clone()
            .ok_or(engine::EditError::Unsupported(
                "project library is unavailable",
            ))?;
        let mut library = library.lock().map_err(|_| engine::RuntimeError::Poisoned)?;
        Ok(run(&mut library)?)
    }
    fn set(&self, library: Option<Arc<Mutex<ProjectLibrary>>>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = library;
        }
    }
}

pub(super) enum Request {
    List,
    ListTrash,
    Create {
        id: ProjectId,
        base_name: String,
        fps: u32,
    },
    Rename {
        id: ProjectId,
        name: String,
    },
    Duplicate {
        sources: Vec<ProjectId>,
    },
    Delete {
        ids: Vec<ProjectId>,
    },
    Restore {
        ids: Vec<ProjectId>,
    },
    Purge {
        ids: Vec<ProjectId>,
    },
    Thumbnail {
        id: ProjectId,
    },
}

pub(super) enum Event {
    Failed(engine::control::Error),
    /// The directories the last listing skipped, each with its reason.
    Unreadable(Vec<UnreadableProject>),
    Ready(Result<Vec<ProjectEntry>, String>),
    Listed(Result<Vec<ProjectEntry>, String>),
    TrashListed(Result<Vec<ProjectEntry>, String>),
    Created(Result<(PathBuf, Vec<ProjectEntry>), String>),
    Renamed(Result<Vec<ProjectEntry>, String>),
    Duplicated {
        result: Result<(), String>,
    },
    DuplicateFinished {
        failures: Vec<String>,
        entries: Result<Vec<ProjectEntry>, String>,
    },
    Deleted {
        failures: Vec<String>,
        entries: Result<Vec<ProjectEntry>, String>,
    },
    Restored {
        failures: Vec<String>,
        trash: Result<Vec<ProjectEntry>, String>,
        entries: Result<Vec<ProjectEntry>, String>,
    },
    Purged {
        failures: Vec<String>,
        trash: Result<Vec<ProjectEntry>, String>,
    },
    Thumbnail(Result<Vec<ProjectEntry>, String>),
}

pub(super) fn next_id() -> ProjectId {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let value = u64::try_from(epoch)
        .unwrap_or(u64::MAX)
        .wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed));
    ProjectId(value)
}

pub(super) fn unique_name(entries: &[ProjectEntry], base: &str) -> String {
    if !entries.iter().any(|entry| entry.name.as_str() == base) {
        return base.to_owned();
    }
    for number in 2..=u64::from(u32::MAX) {
        let candidate = format!("{base} {number}");
        if !entries.iter().any(|entry| entry.name.as_str() == candidate) {
            return candidate;
        }
    }
    format!("{base} {}", u64::MAX)
}

pub(super) fn catalog_worker(
    root: PathBuf,
    budget: ByteBudget,
    corrections: engine::footage::Lookup,
    requests: Receiver<Request>,
    events: wake::Sender<Event>,
    catalog: Catalog,
) {
    let library = match ProjectLibrary::open(&root, budget, corrections) {
        Ok(library) => library,
        Err(error) => {
            let _ = events.send(Event::Ready(Err(error.to_string())));
            return;
        }
    };
    let initial = library.list().map_err(|error| error.to_string());
    let library = Arc::new(Mutex::new(library));
    catalog.set(Some(library.clone()));
    let _ = events.send(Event::Ready(initial));
    if let Ok(library) = library.lock() {
        send_unreadable(&library, &events);
    }
    while let Ok(request) = requests.recv() {
        // Each request holds the library for its own duration; the shorts window's runs
        // take turns with it.
        let Ok(mut library) = library.lock() else {
            break;
        };
        match request {
            Request::List => {
                let listed = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::Listed(listed));
            }
            Request::ListTrash => {
                let listed = library.list_trash().map_err(|error| error.to_string());
                let _ = events.send(Event::TrashListed(listed));
            }
            Request::Create { id, base_name, fps } => {
                let result = (|| {
                    let entries = library.list()?;
                    // A directory the library cannot read still owns its ID.
                    let mut id = id;
                    let taken: BTreeSet<u64> = library
                        .list_unreadable()?
                        .iter()
                        .map(|entry| entry.id.0)
                        .chain(entries.iter().map(|entry| entry.id.0))
                        .collect();
                    while taken.contains(&id.0) {
                        id = next_id();
                    }
                    let name = unique_name(&entries, &base_name);
                    library.execute(
                        id,
                        Command::CreateProject {
                            id,
                            name: name.as_str().into(),
                            fps,
                        },
                    )?;
                    let entries = library.list()?;
                    let path = entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .map(|entry| entry.path.clone())
                        .ok_or(engine::control::Error::Edit(engine::EditError::NoItem(
                            project::ItemId(0),
                        )))?;
                    Ok((path, entries))
                })()
                .map_err(|error: engine::control::Error| error.to_string());
                let _ = events.send(Event::Created(result));
            }
            Request::Rename { id, name } => {
                let result = (|| {
                    library.execute(
                        id,
                        Command::RenameProject {
                            name: name.as_str().into(),
                        },
                    )?;
                    library.list()
                })()
                .map_err(|error: engine::library::Error| error.to_string());
                let _ = events.send(Event::Renamed(result));
            }
            Request::Duplicate { sources } => {
                let mut failures = Vec::new();
                for source in sources {
                    let result = (|| {
                        let entries = library.list()?;
                        let next = entries
                            .iter()
                            .map(|entry| entry.id.0)
                            .chain(library.list_unreadable()?.iter().map(|entry| entry.id.0))
                            .max()
                            .unwrap_or(0)
                            .checked_add(1)
                            .ok_or(engine::control::Error::Control(
                                engine::control::ControlError::Limit,
                            ))?;
                        library.execute(
                            source,
                            Command::DuplicateProject {
                                id: ProjectId(next),
                            },
                        )?;
                        Ok(())
                    })()
                    .map_err(|error: engine::control::Error| error.to_string());
                    if let Err(error) = &result {
                        failures.push(error.clone());
                    }
                    let _ = events.send(Event::Duplicated { result });
                }
                let entries = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::DuplicateFinished { failures, entries });
            }
            Request::Delete { ids } => {
                let mut failures = Vec::new();
                for id in ids {
                    if let Err(error) = library.execute(id, Command::DeleteProject {}) {
                        failures.push(error.to_string());
                    }
                }
                let entries = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::Deleted { failures, entries });
            }
            Request::Restore { ids } => {
                let mut failures = Vec::new();
                for id in ids {
                    if let Err(error) = library.execute(id, Command::RestoreProject {}) {
                        failures.push(error.to_string());
                    }
                }
                let trash = library.list_trash().map_err(|error| error.to_string());
                let entries = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::Restored {
                    failures,
                    trash,
                    entries,
                });
            }
            Request::Purge { ids } => {
                let mut failures = Vec::new();
                for id in ids {
                    if let Err(error) = library.execute(id, Command::PurgeProject {}) {
                        failures.push(error.to_string());
                    }
                }
                let trash = library.list_trash().map_err(|error| error.to_string());
                let _ = events.send(Event::Purged { failures, trash });
            }
            Request::Thumbnail { id } => {
                let result = library
                    .execute(id, Command::ThumbnailProject {})
                    .and_then(|_| library.list())
                    .map_err(|error| error.to_string());
                let _ = events.send(Event::Thumbnail(result));
            }
        }
        send_unreadable(&library, &events);
    }
    catalog.set(None);
}

/// Report the directories the listing skipped; a failed scan keeps the previous report.
fn send_unreadable(library: &ProjectLibrary, events: &wake::Sender<Event>) {
    if let Ok(unreadable) = library.list_unreadable() {
        let _ = events.send(Event::Unreadable(unreadable));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_follow_the_created_order_and_stop_at_a_free_suffix() {
        let root = tempfile::tempdir().expect("temporary directory");
        let mut library = ProjectLibrary::open(
            root.path(),
            ByteBudget::new(64 << 20).expect("budget"),
            engine::footage::uncorrected(),
        )
        .expect("library");
        for (id, name) in [(1u64, "New project"), (2, "New project 2"), (3, "Other")] {
            library
                .execute(
                    ProjectId(id),
                    Command::CreateProject {
                        id: ProjectId(id),
                        name: name.into(),
                        fps: 30,
                    },
                )
                .expect("create");
        }
        let entries = library.list().expect("list");
        assert_eq!(unique_name(&entries, "New project"), "New project 3");
        assert_eq!(unique_name(&entries, "Other"), "Other 2");
    }
}

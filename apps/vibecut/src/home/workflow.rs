//! Runs the managed catalog's requests on one worker thread.
//!
//! The worker owns the exclusive [`ProjectLibrary`] handle; every request is a
//! registered engine command, so no filesystem call blocks the UI thread.

use engine::{
    ByteBudget, Command,
    library::{ProjectEntry, ProjectLibrary},
};
use project::ProjectId;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, Sender},
    },
};

pub(super) enum Request {
    List,
    ListTrash,
    Create { id: ProjectId, base_name: String },
    Rename { id: ProjectId, name: String },
    Duplicate { sources: Vec<ProjectId> },
    Delete { ids: Vec<ProjectId> },
    Restore { ids: Vec<ProjectId> },
    Purge { ids: Vec<ProjectId> },
    Thumbnail { id: ProjectId },
}

pub(super) enum Event {
    Failed(engine::EditError),
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
    requests: Receiver<Request>,
    events: Sender<Event>,
) {
    let mut library = match ProjectLibrary::open(&root, budget) {
        Ok(library) => library,
        Err(error) => {
            let _ = events.send(Event::Ready(Err(error.to_string())));
            return;
        }
    };
    let initial = library.list().map_err(|error| error.to_string());
    let _ = events.send(Event::Ready(initial));
    while let Ok(request) = requests.recv() {
        match request {
            Request::List => {
                let listed = library.list().map_err(|error| error.to_string());
                let _ = events.send(Event::Listed(listed));
            }
            Request::ListTrash => {
                let listed = library.list_trash().map_err(|error| error.to_string());
                let _ = events.send(Event::TrashListed(listed));
            }
            Request::Create { id, base_name } => {
                let result = (|| {
                    let entries = library.list()?;
                    let name = unique_name(&entries, &base_name);
                    library.execute(
                        id,
                        Command::CreateProject {
                            id,
                            name: name.as_str().into(),
                        },
                    )?;
                    let entries = library.list()?;
                    let path = entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .map(|entry| entry.path.clone())
                        .ok_or(engine::EditError::NoItem(project::ItemId(0)))?;
                    Ok((path, entries))
                })()
                .map_err(|error: engine::EditError| error.to_string());
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
                .map_err(|error: engine::EditError| error.to_string());
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
                            .max()
                            .unwrap_or(0)
                            .checked_add(1)
                            .ok_or(engine::EditError::Limit)?;
                        library.execute(
                            source,
                            Command::DuplicateProject {
                                id: ProjectId(next),
                            },
                        )?;
                        Ok(())
                    })()
                    .map_err(|error: engine::EditError| error.to_string());
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
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_follow_the_created_order_and_stop_at_a_free_suffix() {
        let root = tempfile::tempdir().expect("temporary directory");
        let mut library =
            ProjectLibrary::open(root.path(), ByteBudget::new(64 << 20).expect("budget"))
                .expect("library");
        for (id, name) in [(1u64, "New project"), (2, "New project 2"), (3, "Other")] {
            library
                .execute(
                    ProjectId(id),
                    Command::CreateProject {
                        id: ProjectId(id),
                        name: name.into(),
                    },
                )
                .expect("create");
        }
        let entries = library.list().expect("list");
        assert_eq!(unique_name(&entries, "New project"), "New project 3");
        assert_eq!(unique_name(&entries, "Other"), "Other 2");
    }
}

//! The shorts window's runs, kept in the managed project library the catalog worker holds.
use crate::{
    home::Catalog,
    shorts::{RunEntry, RunStore},
};
use engine::control::Result;
use project::{Project, ProjectId, Thumbnail};
use std::cmp::Reverse;

/// The one library of the process, lent to the shorts window from the composition root. A run
/// is an ordinary managed project that carries its `shorts_run`; the library lists it apart
/// from the editor projects.
pub(super) struct CatalogRuns(pub(super) Catalog);

impl RunStore for CatalogRuns {
    fn list(&self) -> Result<Vec<RunEntry>> {
        self.0.with(|library| {
            let mut entries = library.list_runs()?;
            entries.sort_by_key(|entry| Reverse(entry.modified_unix_ms));
            Ok(entries
                .into_iter()
                .filter_map(|entry| {
                    let card = entry.shorts?;
                    Some(RunEntry {
                        id: entry.id,
                        source_name: card.source_name.as_str().to_owned(),
                        clips: card.clips,
                        poster: entry.thumbnail,
                        modified_unix_ms: entry.modified_unix_ms,
                    })
                })
                .collect())
        })
    }
    fn save(&self, project: Project, poster: Thumbnail) -> Result<ProjectId> {
        self.0.with(|library| library.save_run(&project, poster))
    }
    fn load(&self, id: ProjectId) -> Result<Project> {
        self.0.with(|library| library.load_run(id))
    }
    fn remove(&self, id: ProjectId) -> Result<()> {
        self.0.with(|library| library.remove_run(id))
    }
}

//! Holds the trusted command-line grants that bound every MCP client request.
use crate::Error;
use engine::{Command, EditError, GrantClass, control::Policy};
use std::path::{Path, PathBuf};

/// Trusted command-line grants. A client request can never broaden them.
#[derive(Clone, Default)]
pub struct Grants {
    pub(super) output: Option<PathBuf>,
    selected_output: Option<PathBuf>,
}
impl Grants {
    /// True when the human selected an output directory.
    pub fn output_granted(&self) -> bool {
        self.output.is_some()
    }

    /// Admit a pre-existing output directory. Overwrites and imported paths remain refused.
    pub fn output(directory: &Path) -> Result<Self, Error> {
        let output = std::fs::canonicalize(directory)?;
        if !output.is_dir() {
            return Err(Error::Rejected("output directory is not a directory"));
        }
        Ok(Self {
            output: Some(output),
            selected_output: Some(if directory.is_absolute() {
                directory.to_owned()
            } else {
                std::env::current_dir()?.join(directory)
            }),
        })
    }

    fn output_path(
        &self,
        path: &Path,
        overwrite: bool,
        project: &project::Project,
    ) -> engine::Result<()> {
        let root = self.output.as_ref().ok_or(EditError::Permission)?;
        if overwrite || !path.is_absolute() || path.file_name().is_none() {
            return Err(EditError::Permission);
        }
        // A directory-publishing command (chapter sidecars) names its files itself;
        // the client may only select the one granted directory.
        if path.is_dir() {
            let selected = std::fs::canonicalize(path).map_err(|_| EditError::Permission)?;
            return if &selected == root {
                Ok(())
            } else {
                Err(EditError::Permission)
            };
        }
        let parent = path.parent().ok_or(EditError::Permission)?;
        // Only resolve a directory explicitly selected by the human.
        // Resolving an arbitrary client parent first could touch an
        // ungranted filesystem or initiate a Windows UNC connection.
        if parent != root && Some(parent) != self.selected_output.as_deref() {
            return Err(EditError::Permission);
        }
        let parent = std::fs::canonicalize(parent).map_err(|_| EditError::Permission)?;
        if &parent != root {
            return Err(EditError::Permission);
        }
        // An output must not replace even a currently missing source path.
        if project
            .data()
            .assets
            .iter()
            .any(|asset| asset.path() == path)
        {
            return Err(EditError::Permission);
        }
        Ok(())
    }
}
impl Policy for Grants {
    /// The class comes from the command's declaration; this adapter only maps each
    /// class to its trusted grant. Source paths stay human-established capabilities:
    /// a caller-provided SourceAsset would otherwise be a file-read primitive.
    fn authorize(&self, command: &Command, project: &project::Project) -> engine::Result<()> {
        match command.grant() {
            Some(GrantClass::Agent) => Ok(()),
            Some(GrantClass::Output) => {
                let (path, overwrite) = command.published_path().ok_or(EditError::Permission)?;
                self.output_path(&path, overwrite, project)
            }
            Some(GrantClass::Human) | None => Err(EditError::Permission),
        }
    }
}

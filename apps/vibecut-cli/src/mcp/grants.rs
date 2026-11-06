//! Holds the trusted command-line grants that bound every MCP client request.
use crate::args::CliError;
use engine::{Command, EditError, control::Policy};
use std::path::{Path, PathBuf};

/// Trusted command-line grants. A client request can never broaden them.
#[derive(Clone, Default)]
pub struct Grants {
    pub(super) output: Option<PathBuf>,
    selected_output: Option<PathBuf>,
}
impl Grants {
    /// Admit a pre-existing output directory. Overwrites and imported paths remain refused.
    pub fn output(directory: &Path) -> Result<Self, CliError> {
        let output = std::fs::canonicalize(directory)?;
        if !output.is_dir() {
            return Err(CliError::Rejected("output directory is not a directory"));
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
    fn authorize(&self, command: &Command, project: &project::Project) -> engine::Result<()> {
        match command {
            // Source paths are capabilities established by the human opening the project.
            // Caller-provided SourceAsset would otherwise become a file-read primitive.
            Command::AddAsset { .. }
            | Command::CreateProject { .. }
            | Command::DuplicateProject { .. }
            | Command::DeleteProject {}
            | Command::ThumbnailProject {}
            | Command::BeginGesture {}
            | Command::CommitGesture {}
            | Command::CancelGesture {} => Err(EditError::Permission),
            Command::Export {
                path, overwrite, ..
            }
            | Command::ExportSubtitles {
                path, overwrite, ..
            }
            | Command::BurnSubtitles {
                path, overwrite, ..
            } => self.output_path(path, *overwrite, project),
            Command::Snapshot { path, .. } => self.output_path(path, false, project),
            // Each command is reviewed, even within an existing effect class.
            Command::CreateScene { .. }
            | Command::DeleteScenes { .. }
            | Command::SwitchScene { .. }
            | Command::RenameScene { .. }
            | Command::RenameProject { .. }
            | Command::SaveProject { .. }
            | Command::SetMaster { .. }
            | Command::SetViewState { .. }
            | Command::RemoveAssets { .. }
            | Command::AddTrack { .. }
            | Command::RemoveTrack { .. }
            | Command::SetTrackMuted { .. }
            | Command::SetTrackHidden { .. }
            | Command::SetTrackMagnet { .. }
            | Command::SetTrackFlags { .. }
            | Command::RenameTrack { .. }
            | Command::ReorderTrack { .. }
            | Command::SetTrackLocked { .. }
            | Command::InsertItem { .. }
            | Command::MoveItem { .. }
            | Command::MoveItems { .. }
            | Command::SplitAt { .. }
            | Command::TrimStart { .. }
            | Command::TrimEnd { .. }
            | Command::Delete { .. }
            | Command::RippleDelete { .. }
            | Command::SetItemParam { .. }
            | Command::SetItemField { .. }
            | Command::SetSpeed { .. }
            | Command::SetCanvasBackground { .. }
            | Command::SetItemParamMany { .. }
            | Command::SetFade { .. }
            | Command::SetLoudness { .. }
            | Command::ClearLoudness { .. }
            | Command::ApplyLoudness { .. }
            | Command::AddTransition { .. }
            | Command::MoveTransition { .. }
            | Command::SetTransitionDuration { .. }
            | Command::RemoveTransition { .. }
            | Command::AddMask { .. }
            | Command::RemoveMask { .. }
            | Command::UpdateMask { .. }
            | Command::ReorderMasks { .. }
            | Command::AddEffect { .. }
            | Command::RemoveEffect { .. }
            | Command::UpdateEffectParam { .. }
            | Command::ReorderEffects { .. }
            | Command::ToggleEffect { .. }
            | Command::SetAnimatedParam { .. }
            | Command::AddKeyframe { .. }
            | Command::RemoveKeyframe { .. }
            | Command::RetimeKeyframe { .. }
            | Command::SetKeyframeHandle { .. }
            | Command::ScaleKeyframeTime { .. }
            | Command::Retranslate { .. }
            | Command::AddBookmark { .. }
            | Command::MoveBookmark { .. }
            | Command::Batch { .. }
            | Command::Undo { .. }
            | Command::Redo { .. }
            | Command::Inspect { .. }
            | Command::Preview { .. }
            | Command::JobStatus { .. }
            | Command::JobCancel { .. }
            | Command::Catalog { .. }
            | Command::ProviderCatalog { .. }
            | Command::FontsList { .. }
            | Command::FontsLoad { .. } => Ok(()),
            // New commands, effects and source paths have no implicit capability.
            _ => Err(EditError::Permission),
        }
    }
}

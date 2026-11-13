//! Keybinding profile import and export for the file picked by the window.
//! Import goes through the undoable `keybindings` preference edit; a refused
//! file changes nothing.
use super::settings::ShortcutNotice;
use crate::{
    Error, Result,
    editor::Editor,
    input::{ImportMode, KeybindingProfile, MAX_PROFILE_BYTES},
};
use std::path::Path;

/// Apply a picked profile over the current bindings, merging or replacing.
pub(in crate::editor) fn import(editor: &mut Editor, path: &Path, replace: bool) -> Result<()> {
    if std::fs::metadata(path)?.len() > MAX_PROFILE_BYTES as u64 {
        return Err(Error::Invalid("shortcut file"));
    }
    let bytes = std::fs::read(path)?;
    let profile = KeybindingProfile::import(&bytes).map_err(|_| Error::Invalid("shortcut file"))?;
    let mode = if replace {
        ImportMode::Replace
    } else {
        ImportMode::Merge
    };
    let outcome = editor
        .preferences
        .keybindings
        .apply_profile(profile, mode, |id| crate::actions::action(id).is_some())
        .map_err(|_| Error::Invalid("shortcut file"))?;
    let unknown = outcome.unknown_actions.len();
    let value = serde_json::to_value(&outcome.preferences)?;
    editor.desk().set_preference_result("keybindings", value)?;
    editor.dialogs.preferences.shortcut_notice = Some(ShortcutNotice::Imported { unknown });
    Ok(())
}

/// Write the canonical, versioned binding map to the picked destination.
pub(in crate::editor) fn export(editor: &mut Editor, path: &Path) -> Result<()> {
    let bytes = KeybindingProfile::export(editor.preferences.keybindings.clone())
        .map_err(|_| Error::Invalid("shortcut profile"))?;
    std::fs::write(path, bytes)?;
    editor.dialogs.preferences.shortcut_notice = Some(ShortcutNotice::Exported);
    Ok(())
}

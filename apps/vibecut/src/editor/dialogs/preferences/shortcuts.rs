//! Keybinding profile import and export for the file picked by the window.
//! Import goes through the undoable `keybindings` preference edit; a refused
//! file changes nothing.
use super::{State, settings::ShortcutNotice};
use crate::{
    Error, Result,
    editor::Port,
    input::{ImportMode, KeybindingProfile, MAX_PROFILE_BYTES},
};
use std::path::Path;

/// Apply a picked profile over the current bindings, merging or replacing.
pub(in crate::editor) fn import(
    state: &mut State,
    port: &mut Port<'_>,
    path: &Path,
    replace: bool,
) -> Result<()> {
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
    let outcome = port
        .read
        .preferences
        .keybindings
        .apply_profile(profile, mode, |id| crate::actions::action(id).is_some())
        .map_err(|_| Error::Invalid("shortcut file"))?;
    let unknown = outcome.unknown_actions.len();
    let value = serde_json::to_value(&outcome.preferences)?;
    port.set_preference_result("keybindings", value)?;
    state.shortcut_notice = Some(ShortcutNotice::Imported { unknown });
    Ok(())
}

/// Write the canonical, versioned binding map to the picked destination.
pub(in crate::editor) fn export(state: &mut State, port: &Port<'_>, path: &Path) -> Result<()> {
    let bytes = KeybindingProfile::export(port.read.preferences.keybindings.clone())
        .map_err(|_| Error::Invalid("shortcut profile"))?;
    std::fs::write(path, bytes)?;
    state.shortcut_notice = Some(ShortcutNotice::Exported);
    Ok(())
}

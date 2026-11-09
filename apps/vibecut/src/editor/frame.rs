//! Owns editor frame composition.
mod assets;
#[cfg(test)]
mod bench;
mod desk;
mod input;
mod layout;
mod route;
mod shortcuts;
mod topbar;
mod view;
mod workflow;

pub(crate) use desk::{Desk, Request};

use crate::{
    egui,
    i18n::{Locale, Text},
};

/// Read-only composition inputs shared by editor areas.
pub(crate) struct Context<'a> {
    pub(crate) playback: &'a crate::playback::Playback,
    pub(crate) keybindings: &'a crate::actions::KeybindingPreferences,
    pub(crate) session: &'a engine::Session,
    pub(crate) selection: &'a std::collections::BTreeSet<project::ItemId>,
    pub(crate) locale: Locale,
    pub(crate) dark: bool,
    pub(crate) preferences: &'a crate::preferences::Preferences,
    /// What the selection currently scopes: items, mask points or keyframes.
    pub(crate) scope: crate::selection::SelectionScope,
    /// The selected keyframes, flat across items and channels.
    pub(crate) keyframes: &'a std::collections::BTreeSet<crate::selection::KeyframeSelection>,
    pub(super) save_status: Text,
    /// Playback position of the shared transport.
    pub(crate) playhead: time::Tick,
    pub(crate) ripple: bool,
    pub(crate) thumbnails: Thumbnails<'a>,
    /// Registered action ids the editor currently refuses.
    pub(crate) disabled: std::collections::BTreeSet<&'static str>,
    /// While an effect is being dragged: the clips that accept it.
    pub(crate) effect_targets: Option<std::collections::BTreeSet<project::ItemId>>,
}
impl Context<'_> {
    /// Tooltip text from the action's first effective persisted chord.
    pub(crate) fn shortcut(&self, action: &str) -> Option<String> {
        shortcut_text(&self.preferences.keybindings, action)
    }
    /// One frame of the canvas rate, at least one tick.
    pub(crate) fn frame_ticks(&self) -> time::Tick {
        frame_ticks(self.session)
    }
    /// The project the timeline draws: a staged preview when one is open,
    /// otherwise the committed snapshot. Other areas keep the committed view.
    pub(crate) fn staged_project(&self) -> &std::sync::Arc<project::Project> {
        self.session
            .preview_project()
            .unwrap_or_else(|| self.session.project())
    }
}

/// Tooltip text from an action's first effective chord.
pub(super) fn shortcut_text(
    bindings: &crate::actions::KeybindingPreferences,
    action: &str,
) -> Option<String> {
    let spec = crate::actions::action(action)?;
    let chords = bindings.effective_chords(spec);
    let chord = crate::actions::Chord::parse(chords.first()?).ok()?;
    Some(chord.display(crate::actions::Platform::current()))
}

/// One frame of the project's canvas rate, at least one tick.
pub(super) fn frame_ticks(session: &engine::Session) -> time::Tick {
    session
        .project()
        .data()
        .canvas
        .fps
        .frame_duration()
        .ok()
        .filter(|frame| *frame > time::Tick::ZERO)
        .unwrap_or(time::Tick(1))
}

/// Decoded thumbnails shared by every area, uploaded to the GPU on first use.
pub(crate) struct Thumbnails<'a> {
    pub(super) images:
        &'a std::collections::BTreeMap<media::AssetId, engine::thumbnails::RetainedThumbnail>,
    pub(super) textures:
        &'a std::cell::RefCell<std::collections::BTreeMap<media::AssetId, egui::TextureHandle>>,
}
impl Thumbnails<'_> {
    /// Upload one decoded thumbnail on first use; later frames reuse the handle.
    pub(crate) fn texture(
        &self,
        ctx: &egui::Context,
        id: media::AssetId,
    ) -> Option<egui::TextureHandle> {
        let mut textures = self.textures.borrow_mut();
        if let Some(texture) = textures.get(&id) {
            return Some(texture.clone());
        }
        let thumbnail = self.images.get(&id)?;
        // One retained image pays for one CPU image and GPU upload, even when
        // several library records refer to the same content/frame.
        if let Some(texture) = textures.iter().find_map(|(other, texture)| {
            self.images
                .get(other)
                .filter(|image| image.shares_pixels(thumbnail))
                .map(|_| texture.clone())
        }) {
            textures.insert(id, texture.clone());
            return Some(texture);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [thumbnail.width as usize, thumbnail.height as usize],
            &thumbnail.pixels,
        );
        let texture = ctx.load_texture(
            format!("media.thumbnail.{}", id.0),
            image,
            egui::TextureOptions::LINEAR,
        );
        textures.insert(id, texture.clone());
        Some(texture)
    }
}

impl Context<'_> {
    pub(crate) fn tr(&self, key: Text) -> &'static str {
        crate::i18n::text(self.locale, key)
    }
}

/// Transient editor-window chrome, independent of every other window.
#[derive(Default)]
pub(super) struct State {
    pub(super) layout_drag: Option<[f32; 3]>,
    pub(super) rename: Option<String>,
    pub(super) rename_focus: bool,
    widgets: Vec<(&'static str, Option<&'static str>, egui::Response)>,
}

pub(super) enum Action {
    Activate(&'static str),
    Rename(String),
}

pub(super) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    topbar::show(state, cx, ui)
}

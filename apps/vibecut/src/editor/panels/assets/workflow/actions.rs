//! Handles the assets area's own actions through the port.
use super::super::{Action, AssetsTab, AudioSection, egui};
use super::State;
use crate::editor::{Port, Request};

impl State {
    /// Start-of-frame work: open the first Music page and report a finished clip export.
    pub(in crate::editor) fn begin_frame(&mut self, port: &mut Port<'_>) {
        if port.read.preferences.assets_tab == AssetsTab::Sounds
            && self.audio_section == AudioSection::Music
        {
            self.music_loaded();
        }
        if let Some((text, detail)) = self.poll_clip_exports() {
            let message = port.tr(text);
            port.toast(message, detail);
        }
    }

    /// Apply what the view returned. `ui` is present when the actions came from a painted
    /// surface; the marquee and the captions panel need it, the dialogs do not.
    pub(in crate::editor) fn apply(
        &mut self,
        port: &mut Port<'_>,
        mut ui: Option<&mut egui::Ui>,
        actions: Vec<Action>,
    ) {
        for action in actions {
            match action {
                Action::SelectTab(tab) => {
                    if let Err(error) = self.remember_assets_tab(port, tab) {
                        port.fail(error.to_string());
                    }
                }
                Action::Command(id) => port.request(Request::Action(id)),
                Action::Select(id, modifiers) => self.select_asset(port, id, modifiers),
                Action::Delete(ids) => self.request_media_delete(port.session, ids),
                Action::ExportClips(ids) => match self.queue_clip_export(port.read.budget, ids) {
                    Ok(()) => port.request(Request::ExportClips),
                    Err(error) => port.fail(error.to_string()),
                },
                Action::ClipExportCancel => {
                    if let Err(error) = self.cancel_clip_export(port.session) {
                        port.fail(error.to_string());
                    }
                }
                Action::Record(widget) => {
                    let widget = *widget;
                    let icon = widget.glyph.map(|glyph| glyph.name());
                    port.widget(widget.id, icon, widget.tooltip, &widget.response);
                }
                Action::Marquee {
                    content,
                    background,
                    rects,
                } => {
                    if let Some(ui) = ui.as_deref_mut() {
                        self.media_marquee(port, ui, content, &background, &rects);
                    }
                }
                Action::Captions(rect) => {
                    if let Some(ui) = ui.as_deref_mut() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        self.show_captions(port, &mut child);
                    }
                }
                Action::EffectDrag(kind) => self.effect_drag = Some(kind.to_owned()),
                Action::MediaDrag(ids) => self.begin_media_drag(port, ids),
                Action::AddEffect(kind) => self.add_effect_from_selection(port, kind),
                Action::TransitionDrag(kind) => self.begin_transition_drag(port, kind),
                Action::AddTransition(kind) => self.add_transition_from_selection(port, kind),
                Action::RetryEffect(kind) => self.retry_effect(kind),
                Action::SoundsMode(mode) => self.sounds_set_mode(mode),
                Action::SoundsSearch { more } => self.sounds_search(more),
                Action::SoundsCommercial(on) => self.sounds_set_commercial(on),
                Action::SoundsPreview(id) => self.sounds_preview(id),
                Action::SoundsToggleSaved(id) => self.sounds_toggle_saved(id),
                Action::SoundsAdd(id) => self.sounds_add(port, id),
                Action::SoundsCancelCache => self.sounds_cancel_cache(),
                Action::SoundsRetry => self.sounds_retry(),
                Action::MusicMode(mode) => self.music_set_mode(mode),
                Action::MusicChip(chip) => self.music_set_chip(chip),
                Action::MusicSearch { more } => self.music_search(more),
                Action::MusicPreview(id) => self.music_preview(&id),
                Action::MusicToggleSaved(id) => self.music_toggle_saved(&id),
                Action::MusicAdd(id) => self.music_add(port, &id),
                Action::MusicCancelCache => self.music_cancel_cache(),
                Action::MusicRetry => self.music_retry(),
                Action::MusicRetryAdd => self.music_retry_add(),
                Action::LibraryRetry(store) => self.request_library_reopen(store),
                Action::MusicCredits => {
                    let copied = port.tr(crate::i18n::Text::MusicCreditsCopied);
                    port.toast(copied, None);
                }
                Action::MusicKeyApply => self.music_key_apply(port),
                Action::MusicKeyRemove => self.music_key_remove(port),
                Action::ApplyAdjustment(index) => self.apply_adjustment_preset(port, index),
                Action::DeleteAdjustment(index) => {
                    if let Err(error) = self.delete_adjustment_preset(port, index) {
                        port.fail(error.to_string());
                    }
                }
                Action::InsertSticker(id) => self.insert_sticker(port, &id),
                Action::InsertShape(id) => port.request(Request::InsertShape(id)),
                Action::ClearStickerRecents => self.clear_sticker_recents(port),
                Action::ToggleFavorite {
                    family,
                    id,
                    favorite,
                } => {
                    if let Err(error) = self.toggle_favorite(port, family, &id, favorite) {
                        port.fail(error.to_string());
                    }
                }
            }
        }
    }
}

//! Routes assets intentions through existing editor commands.
use crate::editor::{Editor, assets, captions, egui};

impl Editor {
    pub(in crate::editor) fn show_assets(&mut self, ui: &mut egui::Ui) {
        if self.preferences.assets_tab == crate::panels::AssetsTab::Sounds
            && self.assets.audio_section == assets::AudioSection::Music
        {
            self.music_loaded();
        }
        assets::prepare(self, ui.ctx());
        let mut state = std::mem::take(&mut self.assets);
        let cx = self.context();
        let actions = assets::show(&mut state, &cx, ui);
        drop(cx);
        self.assets = state;
        for action in actions {
            match action {
                assets::Action::SelectTab(tab) => {
                    if let Err(error) = self.remember_assets_tab(tab) {
                        self.desk().fail(error.to_string());
                    }
                }
                assets::Action::Command(id) => self.action(&id),
                assets::Action::Select(id, modifiers) => self.select_asset(id, modifiers),
                assets::Action::Delete(ids) => self.request_media_delete(ids),
                assets::Action::Record(widget) => {
                    self.remember_with_icon(
                        &widget.id,
                        widget.glyph.map(|glyph| glyph.name()),
                        &widget.response,
                    );
                    if let Some(state) = self.frame.widgets.get_mut(&widget.id) {
                        state.tooltip = widget.tooltip;
                    }
                }
                assets::Action::Marquee {
                    content,
                    background,
                    rects,
                } => self.media_marquee(ui, content, &background, &rects),
                assets::Action::Captions(rect) => {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    captions::show(self, &mut child);
                    self.drain_requests();
                }
                assets::Action::EffectDrag(kind) => self.effect_drag = Some(kind.to_owned()),
                assets::Action::AddEffect(kind) => self.add_effect_from_selection(kind),
                assets::Action::TransitionDrag(kind) => self.begin_transition_drag(kind),
                assets::Action::AddTransition(kind) => self.add_transition_from_selection(kind),
                assets::Action::RetryEffect(kind) => {
                    if let Some(definition) = project::effect_definitions()
                        .iter()
                        .find(|definition| definition.id == kind)
                        && let Some(params) = crate::editor::effect_preview_instance(definition)
                        && let Some(previews) = &mut self.effect_previews
                    {
                        previews.retry(kind, &params, &project::Animations::default());
                    }
                }
                assets::Action::SoundsMode(mode) => self.sounds_set_mode(mode),
                assets::Action::SoundsSearch { more } => self.sounds_search(more),
                assets::Action::SoundsCommercial(on) => self.sounds_set_commercial(on),
                assets::Action::SoundsPreview(id) => self.sounds_preview(id),
                assets::Action::SoundsToggleSaved(id) => self.sounds_toggle_saved(id),
                assets::Action::SoundsAdd(id) => self.sounds_add(id),
                assets::Action::SoundsCancelCache => self.sounds_cancel_cache(),
                assets::Action::SoundsRetry => self.sounds_retry(),
                assets::Action::MusicMode(mode) => self.music_set_mode(mode),
                assets::Action::MusicChip(chip) => self.music_set_chip(chip),
                assets::Action::MusicSearch { more } => self.music_search(more),
                assets::Action::MusicPreview(id) => self.music_preview(&id),
                assets::Action::MusicToggleSaved(id) => self.music_toggle_saved(&id),
                assets::Action::MusicAdd(id) => self.music_add(&id),
                assets::Action::MusicCancelCache => self.music_cancel_cache(),
                assets::Action::MusicRetry => self.music_retry(),
                assets::Action::MusicRetryAdd => self.music_retry_add(),
                assets::Action::MusicCredits => {
                    let copied = crate::i18n::text(
                        self.preferences.locale,
                        crate::i18n::Text::MusicCreditsCopied,
                    )
                    .to_owned();
                    self.set_toast(&copied, None);
                }
                assets::Action::MusicKeyApply => self.music_key_apply(),
                assets::Action::MusicKeyRemove => self.music_key_remove(),
            }
        }
    }
}

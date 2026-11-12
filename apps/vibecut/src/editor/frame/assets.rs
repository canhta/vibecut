//! Routes assets intentions through existing editor commands.
use crate::editor::{Editor, Rect, Vec2, assets, captions, egui};

impl Editor {
    pub(in crate::editor) fn show_assets(&mut self, ui: &mut egui::Ui) {
        if self.preferences.assets_tab == assets::AssetsTab::Sounds
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
        self.apply_assets_actions(Some(ui), actions);
    }
    fn apply_assets_actions(
        &mut self,
        mut ui: Option<&mut egui::Ui>,
        actions: Vec<assets::Action>,
    ) {
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
                } => {
                    if let Some(ui) = ui.as_deref_mut() {
                        self.media_marquee(ui, content, &background, &rects);
                    }
                }
                assets::Action::Captions(rect) => {
                    if let Some(ui) = ui.as_deref_mut() {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        captions::show(self, &mut child);
                        self.drain_requests();
                    }
                }
                assets::Action::EffectDrag(kind) => self.assets.effect_drag = Some(kind.to_owned()),
                assets::Action::AddEffect(kind) => {
                    self.with_assets(|assets, desk| assets.add_effect_from_selection(desk, kind));
                }
                assets::Action::TransitionDrag(kind) => {
                    self.with_assets(|assets, desk| assets.begin_transition_drag(desk, kind));
                }
                assets::Action::AddTransition(kind) => {
                    self.with_assets(|assets, desk| {
                        assets.add_transition_from_selection(desk, kind)
                    });
                }
                assets::Action::RetryEffect(kind) => self.assets.retry_effect(kind),
                assets::Action::SoundsMode(mode) => self.assets.sounds_set_mode(mode),
                assets::Action::SoundsSearch { more } => self.assets.sounds_search(more),
                assets::Action::SoundsCommercial(on) => self.assets.sounds_set_commercial(on),
                assets::Action::SoundsPreview(id) => self.assets.sounds_preview(id),
                assets::Action::SoundsToggleSaved(id) => self.assets.sounds_toggle_saved(id),
                assets::Action::SoundsAdd(id) => {
                    self.with_assets(|assets, desk| assets.sounds_add(desk, id));
                }
                assets::Action::SoundsCancelCache => self.assets.sounds_cancel_cache(),
                assets::Action::SoundsRetry => self.assets.sounds_retry(),
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
                assets::Action::ApplyAdjustment(index) => self.apply_adjustment_preset(index),
                assets::Action::DeleteAdjustment(index) => {
                    if let Err(error) = self.delete_adjustment_preset(index) {
                        self.desk().fail(error.to_string());
                    }
                }
            }
        }
    }
    pub(in crate::editor) fn show_media_delete_dialog(&mut self, ctx: &egui::Context) {
        let mut state = std::mem::take(&mut self.assets);
        let cx = self.context();
        let actions = assets::delete_dialog(&state, &cx, ctx);
        drop(cx);
        self.assets = state;
        self.apply_assets_actions(None, actions);
    }
    pub(in crate::editor) fn show_import_overlay(&mut self, ctx: &egui::Context) {
        let rect = self
            .frame
            .panels
            .get("media")
            .copied()
            .unwrap_or_else(|| Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 200.0)));
        let mut state = std::mem::take(&mut self.assets);
        let cx = self.context();
        let actions = assets::import_overlay(&state, &cx, ctx, rect);
        drop(cx);
        self.assets = state;
        self.apply_assets_actions(None, actions);
    }
    pub(in crate::editor) fn show_drop_overlay(&mut self, ctx: &egui::Context, hovering: bool) {
        let importing = self.assets.importing();
        if !hovering || importing {
            return;
        }
        let cx = self.context();
        assets::drop_overlay(&cx, ctx, hovering, importing);
    }
    pub(in crate::editor) fn show_toast(&mut self, ctx: &egui::Context) {
        let cx = self.context();
        assets::toast(self.toast.as_ref(), &cx, ctx);
    }
}

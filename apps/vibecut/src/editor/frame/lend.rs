//! Lends areas their ports and applies the notes they leave behind.
use crate::editor::panels::player;
use crate::editor::{Editor, SelectionScopes};
use crate::editor::{Note, Port, Read, Rect, Vec2, assets, egui, timeline};

impl Editor {
    /// Lend the assets state and a port to one workflow step, then apply the notes the
    /// step left, in order. Cross-area requests the notes carry run once the state is back.
    pub(in crate::editor) fn with_assets<T>(
        &mut self,
        run: impl FnOnce(&mut assets::State, &mut Port<'_>) -> T,
    ) -> T {
        let insertion_available = self.insertion_available();
        let caption_bank_ready = self.caption_bank_ready();
        let mut state = std::mem::take(&mut self.assets);
        let mut port = Port::new(
            &mut self.session,
            Read {
                budget: self.playback.budget(),
                preferences: &self.preferences,
                playhead: self.playback.position().ok(),
                item_rects: self.timeline.item_rects(),
                now: self.clock.now(),
                insertion_available,
                caption_bank_ready,
            },
        );
        let result = run(&mut state, &mut port);
        let notes = port.into_notes();
        self.assets = state;
        self.apply_notes(notes);
        result
    }

    /// Lend the player state and a port to one step, then apply the notes it left, in
    /// order. Cross-area requests the notes carry run once the state is back.
    pub(in crate::editor) fn with_player<T>(
        &mut self,
        run: impl FnOnce(&mut player::State, &mut Port<'_>) -> T,
    ) -> T {
        let insertion_available = self.insertion_available();
        let caption_bank_ready = self.caption_bank_ready();
        let mut port = Port::new(
            &mut self.session,
            Read {
                budget: self.playback.budget(),
                preferences: &self.preferences,
                playhead: self.playback.position().ok(),
                item_rects: self.timeline.item_rects(),
                now: self.clock.now(),
                insertion_available,
                caption_bank_ready,
            },
        );
        let result = run(&mut self.player, &mut port);
        let notes = port.into_notes();
        self.apply_notes(notes);
        result
    }

    /// Lend the timeline state and a port to one step, then apply the notes the step left,
    /// in order. Cross-area requests the notes carry run once the state is back.
    pub(in crate::editor) fn with_timeline<T>(
        &mut self,
        run: impl FnOnce(&mut timeline::State, &mut Port<'_>) -> T,
    ) -> T {
        let insertion_available = self.insertion_available();
        let caption_bank_ready = self.caption_bank_ready();
        let mut state = std::mem::take(&mut self.timeline);
        let item_rects = state.item_rects().to_vec();
        let mut port = Port::new(
            &mut self.session,
            Read {
                budget: self.playback.budget(),
                preferences: &self.preferences,
                playhead: self.playback.position().ok(),
                item_rects: &item_rects,
                now: self.clock.now(),
                insertion_available,
                caption_bank_ready,
            },
        );
        let result = run(&mut state, &mut port);
        let notes = port.into_notes();
        self.timeline = state;
        self.apply_notes(notes);
        result
    }

    /// Apply what an area asked for, in the order it asked.
    fn apply_notes(&mut self, notes: Vec<Note>) {
        let mut requested = false;
        for note in notes {
            match note {
                Note::Fail(message) => self.error = Some(message),
                Note::Toast { text, detail } => self.set_toast(&text, detail),
                Note::ClearToast => self.toast = None,
                Note::Sync => self.sync_preview(),
                Note::ResetSelection { drafts } => {
                    self.selection_scopes = SelectionScopes::default();
                    if drafts {
                        self.properties.clear_drafts();
                    }
                }
                Note::Preference {
                    key,
                    value,
                    history,
                } => {
                    let result = if history {
                        self.set_preference(&key, value)
                    } else {
                        self.preferences.set(&key, value)
                    };
                    if let Err(error) = result {
                        self.error = Some(error.to_string());
                    }
                }
                Note::Widget {
                    id,
                    icon,
                    tooltip,
                    response,
                } => {
                    self.remember_with_icon(id.clone(), icon, &response);
                    if let (Some(tooltip), Some(state)) = (tooltip, self.frame.widgets.get_mut(&id))
                    {
                        state.tooltip = Some(tooltip);
                    }
                }
                Note::Request(request) => {
                    self.requests.push(request);
                    requested = true;
                }
            }
        }
        if requested {
            self.drain_requests();
        }
    }

    /// Paint the assets surface, then let the area handle the actions it returned.
    pub(in crate::editor) fn show_assets(&mut self, ui: &mut egui::Ui) {
        self.with_assets(|assets, port| assets.begin_frame(port));
        if self.assets.clip_export_status().is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_nanos(1_000_000_000 / 30));
        }
        let insertion_ready = self.insertion_available();
        let now = self.clock.now();
        let mut state = std::mem::take(&mut self.assets);
        let cx = self.context();
        assets::prepare(&mut state, &cx, ui.ctx(), now, insertion_ready);
        let actions = assets::show(&mut state, &cx, ui);
        drop(cx);
        self.assets = state;
        self.with_assets(|assets, port| assets.apply(port, Some(ui), actions));
    }

    pub(in crate::editor) fn show_media_delete_dialog(&mut self, ctx: &egui::Context) {
        let state = std::mem::take(&mut self.assets);
        let cx = self.context();
        let actions = assets::delete_dialog(&state, &cx, ctx);
        drop(cx);
        self.assets = state;
        self.with_assets(|assets, port| assets.apply(port, None, actions));
    }

    pub(in crate::editor) fn show_import_overlay(&mut self, ctx: &egui::Context) {
        let rect = self
            .frame
            .panels
            .get("media")
            .copied()
            .unwrap_or_else(|| Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 200.0)));
        let state = std::mem::take(&mut self.assets);
        let cx = self.context();
        let actions = assets::import_overlay(&state, &cx, ctx, rect);
        drop(cx);
        self.assets = state;
        self.with_assets(|assets, port| assets.apply(port, None, actions));
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

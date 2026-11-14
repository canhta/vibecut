//! Renders the timeline panel.
use crate::i18n;
mod bookmark;
mod clips;
mod easing;
mod hover;
mod lanes;
mod ruler;
mod scenes;
mod toolbar;
mod tracks;
mod transition;

use crate::editor::timeline::{
    Action, Align2, Color32, Context, FontId, State, Text, TimelineLayout, egui, theme,
};

pub(super) use clips::item_label;
#[cfg(test)]
pub(super) use clips::missing_rows;
pub(in crate::editor) use scenes::show_scenes;
pub(in crate::editor) use toolbar::TOOLBAR_ACTIONS;
pub(in crate::editor) use tracks::add_track_command;
pub(super) use tracks::track_height;

/// The theme inks of the panel, resolved once for the active appearance.
#[derive(Clone, Copy)]
pub(in crate::editor::timeline) struct Ink {
    dark: bool,
}
impl Ink {
    pub(in crate::editor::timeline) fn of(cx: &Context<'_>) -> Self {
        Self { dark: cx.dark }
    }
    pub(in crate::editor::timeline) fn dark(self) -> bool {
        self.dark
    }
    pub(in crate::editor::timeline) fn accent(self) -> Color32 {
        theme::accent(self.dark)
    }
    pub(in crate::editor::timeline) fn panel(self) -> Color32 {
        theme::panel(self.dark)
    }
    pub(in crate::editor::timeline) fn field(self) -> Color32 {
        theme::field(self.dark)
    }
    pub(in crate::editor::timeline) fn border(self) -> Color32 {
        theme::border(self.dark)
    }
    pub(in crate::editor::timeline) fn text(self) -> Color32 {
        theme::text(self.dark)
    }
    pub(in crate::editor::timeline) fn secondary(self) -> Color32 {
        theme::secondary(self.dark)
    }
}

pub(in crate::editor::timeline) fn tr(cx: &Context<'_>, key: Text) -> &'static str {
    i18n::text(cx.locale, key)
}

/// Paint the whole panel (toolbar, ruler, headers, lanes, scrollbars) and
/// return what it asks the frame to do.
pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    let layout = TimelineLayout::new(ui.max_rect());
    state.viewport = layout.body.width().max(1.0);
    let duration = cx.staged_project().duration();
    let min = state.min_zoom(duration);
    if state.zoom < min {
        state.apply_zoom(min, duration, cx.playhead);
    }
    state.toolbar(cx, ui, &layout);
    state.scroll_input(cx, ui, &layout);
    state.follow_transport(cx, &layout);
    let views = state.views(cx, &layout);
    state.paint_ruler(cx, ui, &layout, views.bookmarks.as_slice());
    state.advance_bookmark_drag(cx, ui);
    state.paint_tracks(cx, ui, &layout, &views.rows, &views.items);
    state.paint_transitions(cx, ui, &layout, &views.rows);
    state.advance_drag(cx, ui, &layout, &views.rows);
    state.advance_media_drag(cx, ui, &layout, &views.rows);
    let modifiers = ui.ctx().input(|input| input.modifiers);
    state.paint_lanes(cx, ui, &layout, &views.items, modifiers);
    state.advance_keyframe_drag(cx, ui, &layout);
    state.advance_volume_drag(cx, ui, &layout);
    state.advance_fade_drag(ui);
    let empty_items = views.items.is_empty();
    let duration = views.duration;
    let rows = views.rows;
    state.track_hover(cx, ui, &layout);
    state.paint_playhead(cx, ui, &layout);
    state.playhead_interaction(cx, ui, &layout);
    state.paint_hover(cx, ui, &layout);
    state.paint_split_cut(cx, ui, &layout);
    state.scrollbars(cx, ui, &layout, duration, &rows);
    state.paint_refusal(cx, ui, &layout);
    if empty_items && !rows.is_empty() {
        let empty = tr(cx, Text::EmptyTimeline);
        ui.painter().text(
            layout.body.center(),
            Align2::CENTER_CENTER,
            empty,
            FontId::proportional(theme::type_size::CHROME),
            Ink::of(cx).secondary(),
        );
    }
    state.show_bookmark_popover(cx, ui.ctx());
    state.show_graph_popover(cx, ui.ctx());
    for request in cx.visuals.take_requests() {
        state.push(Action::Generate(request));
    }
    std::mem::take(&mut state.outbox)
}

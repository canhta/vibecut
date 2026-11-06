//! Renders the timeline panel.
mod clips;
mod ruler;
mod scenes;
mod toolbar;
mod tracks;

use crate::editor::timeline::{
    Action, Align2, Color32, Context, FontId, State, Text, TimelineLayout, catalog, egui, theme,
};

pub(super) use clips::item_label;
pub(in crate::editor) use scenes::show_scenes;
pub(in crate::editor) use toolbar::TOOLBAR_ACTIONS;
pub(in crate::editor) use tracks::add_track_params;
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
    catalog::text(cx.locale, key)
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
    let duration = cx.session.project().duration();
    let min = state.min_zoom(duration);
    if state.zoom < min {
        state.apply_zoom(min, duration, cx.playhead);
    }
    state.toolbar(cx, ui, &layout);
    state.scroll_input(cx, ui, &layout);
    let views = state.views(cx, &layout);
    state.paint_ruler(cx, ui, &layout, views.bookmarks.as_slice());
    state.advance_bookmark_drag(cx, ui);
    state.paint_tracks(cx, ui, &layout, &views.rows, &views.items);
    state.advance_drag(cx, ui, &layout, &views.rows);
    let empty_items = views.items.is_empty();
    let duration = views.duration;
    let rows = views.rows;
    state.paint_playhead(cx, ui, &layout);
    state.playhead_interaction(cx, ui, &layout);
    state.scrollbars(cx, ui, &layout, duration, &rows);
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
    std::mem::take(&mut state.outbox)
}

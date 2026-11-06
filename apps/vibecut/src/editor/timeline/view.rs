//! Renders the timeline area.
mod clips;
mod ruler;
mod toolbar;
mod tracks;

use super::{Align2, Editor, FontId, Text, TimelineLayout, egui, theme};

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let layout = TimelineLayout::new(ui.max_rect());
    editor.timeline_viewport = layout.body.width().max(1.0);
    let min = editor.min_zoom(editor.timeline_viewport);
    if editor.zoom < min {
        editor.apply_zoom(min);
    }
    editor.timeline_toolbar(ui, &layout);
    editor.timeline_scroll_input(ui, &layout);
    let views = editor.timeline_views(&layout);
    editor.paint_ruler(ui, &layout, views.bookmarks.as_slice());
    editor.advance_bookmark_drag(ui);
    editor.paint_tracks(ui, &layout, &views.rows, &views.items);
    editor.advance_drag(ui, &layout, &views.rows);
    let empty_items = views.items.is_empty();
    let duration = views.duration;
    let rows = views.rows;
    editor.paint_playhead(ui, &layout);
    editor.timeline_playhead_interaction(ui, &layout);
    editor.timeline_scrollbars(ui, &layout, duration, &rows);
    if let Err(error) = editor.remember_view() {
        editor.error = Some(error.to_string());
    }
    if empty_items && !rows.is_empty() {
        let empty = editor.tr(Text::EmptyTimeline);
        ui.painter().text(
            layout.body.center(),
            Align2::CENTER_CENTER,
            empty,
            FontId::proportional(theme::type_size::CHROME),
            editor.secondary(),
        );
    }
}

pub(super) use clips::item_label;
pub(super) use tracks::track_height;

mod scenes;
pub(in crate::editor) use scenes::show_scenes;

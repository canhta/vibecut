//! Renders the Filter section of a selected filter clip.
use super::Panel;
use crate::editor::properties::{INTENSITY_SCRUB, ItemId, Text, egui, read, theme};

impl Panel<'_, '_> {
    /// The Filter section: the clip's look, then its Intensity. Intensity is the clip's
    /// opacity channel, so it edits through the ordinary item parameter path, one undo step per
    /// typed value or scrub gesture, and it keys like opacity.
    pub(super) fn filter_section(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(clip) = read::item(self.cx.session.project(), item) else {
            return;
        };
        if !project::is_filter_clip(clip) {
            return;
        }
        let look = project::filter_look(clip).map(|look| look.name);
        self.panel_section(
            ui,
            "inspector.section.filter",
            "inspector.filter",
            Text::Filters,
            true,
            false,
            |panel, ui| {
                let (row, mut value_ui) = panel.row_value_ui(ui, theme::size::PROPERTY_ROW);
                panel.row_label(ui, row, panel.tr(Text::FilterLook));
                let name = look.unwrap_or_else(|| panel.tr(Text::Filters));
                let response = value_ui.label(
                    egui::RichText::new(name)
                        .size(theme::type_size::BODY)
                        .color(theme::text(panel.cx.dark)),
                );
                panel.remember("inspector.filter.look", &response);
                let editable = !read::channel_has_keys(
                    panel.cx.session.project(),
                    item,
                    INTENSITY_SCRUB.field,
                ) || read::range_of(panel.cx.session.project(), item).is_some_and(
                    |(start, end)| start <= panel.cx.playhead && panel.cx.playhead < end,
                );
                panel.slider_row_in(ui, item, INTENSITY_SCRUB, editable);
            },
        );
    }
}

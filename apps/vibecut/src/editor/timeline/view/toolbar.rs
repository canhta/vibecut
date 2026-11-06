//! Renders timeline actions.

use crate::editor::timeline::{
    BUTTON_ICON, Editor, Icon, Rect, TOOLBAR_SPACING, Text, TimelineLayout, Vec2, egui, icons,
    slider_to_zoom, theme, zoom_to_slider,
};

impl Editor {
    pub(in crate::editor::timeline) fn timeline_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
    ) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("timeline.toolbar")
                .max_rect(layout.toolbar.shrink2(Vec2::splat(theme::space::XS))),
        );
        child.set_clip_rect(layout.toolbar);
        child.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::Timeline))
                    .size(16.0)
                    .strong(),
            );
            ui.spacing_mut().item_spacing.x = TOOLBAR_SPACING;
            if self.icon_only(
                ui,
                "edit.undo",
                Icon::ArrowCounterClockwise,
                self.tr(Text::Undo),
                self.shortcut("edit.undo").as_deref(),
                None,
                true,
            ) {
                self.action("edit.undo");
            }
            if self.icon_only(
                ui,
                "edit.redo",
                Icon::ArrowClockwise,
                self.tr(Text::Redo),
                self.shortcut("edit.redo").as_deref(),
                None,
                true,
            ) {
                self.action("edit.redo");
            }
            if self.icon_only(
                ui,
                "timeline.split",
                Icon::Scissors,
                self.tr(Text::Split),
                self.shortcut("timeline.split").as_deref(),
                None,
                true,
            ) {
                self.action("timeline.split");
            }
            if self.icon_only(
                ui,
                "timeline.delete",
                Icon::Trash,
                self.tr(Text::Delete),
                self.shortcut("timeline.delete").as_deref(),
                None,
                true,
            ) {
                self.action("timeline.delete");
            }
            if self.icon_only(
                ui,
                "timeline.add_track",
                Icon::Plus,
                self.tr(Text::AddTrack),
                None,
                None,
                true,
            ) {
                self.action("timeline.add_track");
            }
            if self.icon_only(
                ui,
                "timeline.snap",
                Icon::Magnet,
                self.tr(Text::Snap),
                self.shortcut("timeline.snap").as_deref(),
                Some(self.preferences.snapping),
                true,
            ) {
                self.action("timeline.snap");
            }
            let magnet = self.session.project().data().scene().timeline.magnet;
            if self.icon_only(
                ui,
                "timeline.magnet",
                Icon::Magnet,
                self.tr(Text::Magnet),
                None,
                Some(magnet),
                true,
            ) {
                self.track_command("track.set_magnet", serde_json::json!({"enabled":!magnet}));
            }
            if self.icon_only(
                ui,
                "timeline.ripple",
                Icon::Waveform,
                self.tr(Text::Ripple),
                None,
                Some(self.ripple),
                true,
            ) {
                self.action("timeline.ripple");
            }
            if self.icon_only(
                ui,
                "timeline.zoom_out",
                Icon::MagnifyingGlassMinus,
                self.tr(Text::ZoomOut),
                None,
                None,
                true,
            ) {
                self.action("timeline.zoom_out");
            }
            let min = self.min_zoom(self.timeline_viewport);
            let mut slider = zoom_to_slider(self.zoom, min);
            ui.spacing_mut().slider_width = 96.0;
            let response = ui.add(
                egui::Slider::new(&mut slider, 0.0..=1.0)
                    .show_value(false)
                    .trailing_fill(true),
            );
            self.remember("timeline.zoom_slider", &response);
            if response.changed() {
                self.apply_zoom(slider_to_zoom(slider, min));
            }
            response.on_hover_text(self.tr(Text::ZoomLevel));
            if self.icon_only(
                ui,
                "timeline.zoom_in",
                Icon::MagnifyingGlassPlus,
                self.tr(Text::ZoomIn),
                None,
                None,
                true,
            ) {
                self.action("timeline.zoom_in");
            }
        });
        let bounds = Rect::from_center_size(layout.toolbar.center(), Vec2::new(178.0, 28.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let name = self
                    .session
                    .project()
                    .data()
                    .scene()
                    .name
                    .as_str()
                    .to_owned();
                let response = ui
                    .add_sized(
                        [140.0, theme::size::CONTROL],
                        egui::Button::new(name).truncate(),
                    )
                    .on_hover_text(self.tr(Text::Scenes));
                self.remember("scene.open", &response);
                if response.clicked() {
                    self.action("scene.open");
                }
                let response = ui
                    .add(egui::Button::image(icons::tinted(
                        Icon::FilmStrip,
                        BUTTON_ICON,
                        self.text_color(),
                    )))
                    .on_hover_text(self.tr(Text::Scenes));
                self.remember("scene.open_list", &response);
                if response.clicked() {
                    self.action("scene.open");
                }
            });
        });
    }
}

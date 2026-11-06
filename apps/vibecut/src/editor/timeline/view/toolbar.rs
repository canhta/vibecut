//! Renders timeline actions.
//!
//! Geometry is the measured 32 px strip: 24×24 icon buttons, 4 px edge inset,
//! 12 px between buttons, 1×16 dividers with 8 px either side (221643 y 574-606).

use crate::editor::timeline::{
    Editor, Icon, Rect, Text, TimelineLayout, Vec2, egui, slider_to_zoom, theme, zoom_to_slider,
};

const EDGE: f32 = 4.0;
const CELL: f32 = theme::size::CONTROL;
const GAP: f32 = 12.0;
const DIVIDER_GAP: f32 = 8.0;
const DIVIDER_HEIGHT: f32 = 16.0;

/// Left-to-right cursor over the toolbar row.
struct Cursor {
    x: f32,
    top: f32,
}
impl Cursor {
    fn cell(&mut self, width: f32) -> Rect {
        let rect = Rect::from_min_size(egui::pos2(self.x, self.top), Vec2::new(width, CELL));
        self.x += width + GAP;
        rect
    }
    fn divider(&mut self, ui: &egui::Ui, color: egui::Color32, gap: f32) {
        // The previous cell already added `GAP`; bring the divider `gap` from it.
        let x = self.x - GAP + gap;
        let center = self.top + CELL / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(x + 0.5, center - DIVIDER_HEIGHT / 2.0),
                egui::pos2(x + 0.5, center + DIVIDER_HEIGHT / 2.0),
            ],
            egui::Stroke::new(1.0, color),
        );
        self.x = x + 1.0 + gap;
    }
}

impl Editor {
    pub(in crate::editor::timeline) fn timeline_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
    ) {
        let bar = layout.toolbar;
        let top = bar.top() + (bar.height() - CELL) / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(bar.left(), bar.bottom() + 0.5),
                egui::pos2(bar.right(), bar.bottom() + 0.5),
            ],
            egui::Stroke::new(1.0, theme::separator(self.preferences.dark)),
        );
        let divider = self.border();
        let mut left = Cursor {
            x: bar.left() + EDGE,
            top,
        };
        let rect = left.cell(CELL);
        self.toolbar_button(
            ui,
            rect,
            "timeline.add_track",
            Icon::Plus,
            Text::AddTrack,
            None,
        );
        left.divider(ui, divider, DIVIDER_GAP);
        for (id, icon, label) in [
            ("edit.undo", Icon::ArrowCounterClockwise, Text::Undo),
            ("edit.redo", Icon::ArrowClockwise, Text::Redo),
            ("timeline.split", Icon::Scissors, Text::Split),
            ("timeline.delete", Icon::Trash, Text::Delete),
        ] {
            let rect = left.cell(CELL);
            self.toolbar_button(ui, rect, id, icon, label, None);
        }

        // Right group, placed from the right edge inwards.
        let mut right = bar.right() - EDGE;
        let zoom_in = Rect::from_min_size(egui::pos2(right - CELL, top), Vec2::splat(CELL));
        right = zoom_in.left() - GAP;
        let slider = Rect::from_min_size(
            egui::pos2(right - theme::size::ZOOM_SLIDER, top),
            Vec2::new(theme::size::ZOOM_SLIDER, CELL),
        );
        right = slider.left() - GAP;
        let zoom_out = Rect::from_min_size(egui::pos2(right - CELL, top), Vec2::splat(CELL));
        right = zoom_out.left() - GAP - DIVIDER_GAP;
        let center = top + CELL / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(right - 0.5, center - DIVIDER_HEIGHT / 2.0),
                egui::pos2(right - 0.5, center + DIVIDER_HEIGHT / 2.0),
            ],
            egui::Stroke::new(1.0, divider),
        );
        right -= 1.0 + DIVIDER_GAP;
        let mut toggles = [Rect::NOTHING; 3];
        for toggle in toggles.iter_mut().rev() {
            *toggle = Rect::from_min_size(egui::pos2(right - CELL, top), Vec2::splat(CELL));
            right = toggle.left() - GAP;
        }
        let scene = Rect::from_min_size(egui::pos2(right - CELL - 1.0, top), Vec2::splat(CELL));

        let name = self
            .session
            .project()
            .data()
            .scene()
            .name
            .as_str()
            .to_owned();
        self.toolbar_scene(ui, scene, &name);
        let magnet = self.session.project().data().scene().timeline.magnet;
        let snapping = self.preferences.snapping;
        let ripple = self.ripple;
        self.toolbar_button(
            ui,
            toggles[0],
            "timeline.snap",
            Icon::Magnet,
            Text::Snap,
            Some(snapping),
        );
        if self.toolbar_button(
            ui,
            toggles[1],
            "timeline.magnet",
            Icon::LinkSimple,
            Text::Magnet,
            Some(magnet),
        ) {
            self.track_command("track.set_magnet", serde_json::json!({"enabled":!magnet}));
        }
        self.toolbar_button(
            ui,
            toggles[2],
            "timeline.ripple",
            Icon::Waveform,
            Text::Ripple,
            Some(ripple),
        );
        self.toolbar_button(
            ui,
            zoom_out,
            "timeline.zoom_out",
            Icon::MagnifyingGlassMinus,
            Text::ZoomOut,
            None,
        );
        self.toolbar_button(
            ui,
            zoom_in,
            "timeline.zoom_in",
            Icon::MagnifyingGlassPlus,
            Text::ZoomIn,
            None,
        );
        let min = self.min_zoom(self.timeline_viewport);
        let mut value = zoom_to_slider(self.zoom, min);
        let response = crate::kit::rail_slider(ui, "timeline.zoom_slider", slider, &mut value);
        if response.changed() {
            self.apply_zoom(slider_to_zoom(value, min));
        }
        self.remember("timeline.zoom_slider", &response);
        response.on_hover_text(self.tr(Text::ZoomLevel));
    }

    /// One icon-only button in `rect`; returns the click for controls that
    /// run their own command (the rest dispatch their registered action).
    fn toolbar_button(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        id: &str,
        icon: Icon,
        label: Text,
        selected: Option<bool>,
    ) -> bool {
        let shortcut = self.shortcut(id);
        let clicked = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                self.icon_only(
                    ui,
                    id,
                    icon,
                    self.tr(label),
                    shortcut.as_deref(),
                    selected,
                    true,
                )
            })
            .inner;
        if clicked && crate::actions::action(id).is_some() {
            self.action(id);
            return false;
        }
        clicked
    }

    fn toolbar_scene(&mut self, ui: &mut egui::Ui, rect: Rect, name: &str) {
        let tooltip = format!("{}: {name}", self.tr(Text::Scenes));
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                crate::kit::icon_button(ui, "scene.open", Icon::FilmStrip, &tooltip, false)
            })
            .inner;
        self.remember_with_icon("scene.open", Some(Icon::FilmStrip.name()), &response);
        if response.clicked() {
            self.action("scene.open");
        }
    }
}

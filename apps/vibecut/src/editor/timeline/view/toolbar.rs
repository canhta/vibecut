//! Renders timeline actions.
//!
//! Geometry is the measured 32 px strip: 24×24 icon buttons, 4 px edge inset,
//! 12 px between buttons, 1×16 dividers with 8 px either side (221643 y 574-606).

use crate::editor::timeline::{
    Action, Context, Icon, Rect, State, Text, TimelineLayout, Vec2, catalog, egui, slider_to_zoom,
    theme,
    view::{Ink, tr},
    zoom_to_slider,
};

const EDGE: f32 = 4.0;
const CELL: f32 = theme::size::CONTROL;
const GAP: f32 = 12.0;
const DIVIDER_GAP: f32 = 8.0;
const DIVIDER_HEIGHT: f32 = 16.0;

/// The registered actions the toolbar can show as unavailable.
pub(in crate::editor) const TOOLBAR_ACTIONS: [&str; 10] = [
    "timeline.add_track",
    "edit.undo",
    "edit.redo",
    "timeline.split",
    "timeline.delete",
    "timeline.snap",
    "timeline.magnet",
    "timeline.ripple",
    "timeline.zoom_out",
    "timeline.zoom_in",
];

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

impl State {
    pub(in crate::editor::timeline) fn toolbar(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
    ) {
        let ink = Ink::of(cx);
        let bar = layout.toolbar;
        let top = bar.top() + (bar.height() - CELL) / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(bar.left(), bar.bottom() + 0.5),
                egui::pos2(bar.right(), bar.bottom() + 0.5),
            ],
            egui::Stroke::new(1.0, theme::separator(ink.dark())),
        );
        let divider = ink.border();
        let mut left = Cursor {
            x: bar.left() + EDGE,
            top,
        };
        let rect = left.cell(CELL);
        self.toolbar_button(
            cx,
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
            self.toolbar_button(cx, ui, rect, id, icon, label, None);
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

        let name = cx.staged_project().data().scene().name.as_str().to_owned();
        self.toolbar_scene(cx, ui, scene, &name);
        let magnet = cx.staged_project().data().scene().timeline.magnet;
        self.toolbar_button(
            cx,
            ui,
            toggles[0],
            "timeline.snap",
            Icon::Magnet,
            Text::Snap,
            Some(cx.preferences.snapping),
        );
        if self.toolbar_button(
            cx,
            ui,
            toggles[1],
            "timeline.magnet",
            Icon::LinkSimple,
            Text::Magnet,
            Some(magnet),
        ) {
            self.push(Action::Command {
                id: "track.set_magnet",
                params: serde_json::json!({"enabled":!magnet}),
            });
        }
        self.toolbar_button(
            cx,
            ui,
            toggles[2],
            "timeline.ripple",
            Icon::Waveform,
            Text::Ripple,
            Some(cx.ripple),
        );
        self.toolbar_button(
            cx,
            ui,
            zoom_out,
            "timeline.zoom_out",
            Icon::MagnifyingGlassMinus,
            Text::ZoomOut,
            None,
        );
        self.toolbar_button(
            cx,
            ui,
            zoom_in,
            "timeline.zoom_in",
            Icon::MagnifyingGlassPlus,
            Text::ZoomIn,
            None,
        );
        let duration = cx.staged_project().duration();
        let min = self.min_zoom(duration);
        let mut value = zoom_to_slider(self.zoom, min);
        let response = crate::kit::zoom_rail(ui, "timeline.zoom_slider", slider, &mut value);
        if response.changed() {
            self.apply_zoom(slider_to_zoom(value, min), duration, cx.playhead);
        }
        self.remember("timeline.zoom_slider", &response);
        response.on_hover_text(tr(cx, Text::ZoomLevel));
    }

    /// One icon-only button in `rect`; returns the click for controls that
    /// run their own command (the rest dispatch their registered action).
    #[allow(clippy::too_many_arguments)]
    fn toolbar_button(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        rect: Rect,
        id: &'static str,
        icon: Icon,
        label: Text,
        selected: Option<bool>,
    ) -> bool {
        let enabled = !cx.disabled.contains(id);
        let label = tr(cx, label);
        let tooltip = cx.shortcut(id).map_or_else(
            || label.to_owned(),
            |key| {
                catalog::format(
                    cx.locale,
                    Text::ActionShortcut,
                    &[("action", label), ("shortcut", &key)],
                )
            },
        );
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    crate::kit::icon_button(ui, id, icon, &tooltip, selected.unwrap_or(false))
                })
                .inner
            })
            .inner;
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &tooltip)
            } else {
                egui::WidgetInfo::labeled(egui::Role::Button, enabled, &tooltip)
            }
        });
        self.remember_with(id, Some(icon.name()), Some(tooltip), &response);
        let clicked = enabled && response.clicked();
        if clicked && crate::actions::action(id).is_some() {
            self.push(Action::Activate(id.to_owned()));
            return false;
        }
        clicked
    }

    fn toolbar_scene(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, rect: Rect, name: &str) {
        let tooltip = format!("{}: {name}", tr(cx, Text::Scenes));
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                crate::kit::icon_button(ui, "scene.open", Icon::FilmStrip, &tooltip, false)
            })
            .inner;
        self.remember_with("scene.open", Some(Icon::FilmStrip.name()), None, &response);
        if response.clicked() {
            self.push(Action::Activate("scene.open".to_owned()));
        }
    }
}

//! Renders timeline actions.
//!
//! Geometry is the measured 32 px strip: 24×24 icon buttons, 4 px edge inset,
//! 12 px between buttons, 1×16 dividers with 8 px either side (221643 y 574-606).

use crate::editor::timeline::{
    Action, Context, Icon, Rect, State, Text, TimelineLayout, Tool, Vec2, egui, icons,
    slider_to_zoom, theme,
    view::{Ink, tr},
    zoom_to_slider,
};
use crate::i18n;

const EDGE: f32 = 4.0;
const CELL: f32 = theme::size::CONTROL;
/// Caret glyph and the strip it sits in, right of the tool glyph: the measured 12 px
/// small-glyph token `theme::icon::PLAY`, not a new value.
const TOOL_CARET: f32 = theme::icon::PLAY;
const GAP: f32 = 12.0;
const DIVIDER_GAP: f32 = 8.0;
const DIVIDER_HEIGHT: f32 = 16.0;

/// The registered actions the toolbar can show as unavailable.
pub(in crate::editor) const TOOLBAR_ACTIONS: [&str; 20] = [
    "timeline.add_track",
    "edit.undo",
    "edit.redo",
    "timeline.split",
    "timeline.split_left",
    "timeline.split_right",
    "timeline.extract_audio",
    "timeline.duplicate",
    "timeline.freeze",
    "timeline.delete",
    "timeline.bookmark",
    "timeline.crop",
    "timeline.flip_horizontal",
    "timeline.flip_vertical",
    "timeline.snap",
    "timeline.magnet",
    "timeline.ripple",
    "timeline.preview_axis",
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
        let tool = left.cell(CELL + TOOL_CARET);
        self.toolbar_tool(cx, ui, tool);
        for (id, icon, label) in [
            ("edit.undo", Icon::ArrowCounterClockwise, Text::Undo),
            ("edit.redo", Icon::ArrowClockwise, Text::Redo),
            ("timeline.split", Icon::Scissors, Text::Split),
            ("timeline.split_left", Icon::ArrowLeft, Text::SplitLeft),
            (
                "timeline.split_right",
                Icon::ArrowsLeftRight,
                Text::SplitRight,
            ),
            (
                "timeline.extract_audio",
                Icon::MusicNotes,
                Text::ExtractAudio,
            ),
            ("timeline.duplicate", Icon::Copy, Text::Duplicate),
        ] {
            let rect = left.cell(CELL);
            self.toolbar_button(cx, ui, rect, id, icon, label, None);
        }
        let freeze = left.cell(CELL);
        self.toolbar_unavailable(
            cx,
            ui,
            freeze,
            "timeline.freeze",
            Icon::Snowflake,
            Text::FreezeFrame,
        );
        let delete = left.cell(CELL);
        self.toolbar_button(
            cx,
            ui,
            delete,
            "timeline.delete",
            Icon::Trash,
            Text::Delete,
            None,
        );
        left.divider(ui, divider, DIVIDER_GAP);
        for (id, icon, label) in [
            (
                "timeline.bookmark",
                Icon::BookmarkSimple,
                Text::BookmarkToggle,
            ),
            ("timeline.crop", Icon::Crop, Text::ActionTimelineCrop),
            (
                "timeline.flip_horizontal",
                Icon::ArrowsInLineHorizontal,
                Text::ActionFlipHorizontal,
            ),
            (
                "timeline.flip_vertical",
                Icon::ArrowsInLineVertical,
                Text::ActionFlipVertical,
            ),
        ] {
            let rect = left.cell(CELL);
            self.toolbar_button(cx, ui, rect, id, icon, label, None);
        }
        let graph = left.cell(CELL);
        self.toolbar_graph(cx, ui, graph);

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
        let mut toggles = [Rect::NOTHING; 4];
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
            self.push(Action::Command(Box::new(engine::Command::SetTrackMagnet {
                enabled: !magnet,
            })));
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
        let preview_axis = cx.preferences.preview_axis;
        self.toolbar_button(
            cx,
            ui,
            toggles[3],
            "timeline.preview_axis",
            Icon::ArrowsInLineVertical,
            if preview_axis {
                Text::PreviewAxisOff
            } else {
                Text::PreviewAxisOn
            },
            Some(preview_axis),
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
                i18n::format(
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

    /// The pointer-tool button: the active tool's glyph with a caret. Its menu lists
    /// both tools with their shortcuts and runs the chosen tool's registered action.
    fn toolbar_tool(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, rect: Rect) {
        let ink = Ink::of(cx);
        let (icon, label) = match self.tool {
            Tool::Select => (Icon::Cursor, Text::ActionTimelineToolSelect),
            Tool::Split => (Icon::Scissors, Text::ActionTimelineToolSplit),
        };
        let label = tr(cx, label);
        let glyph = Rect::from_min_size(rect.min, Vec2::splat(CELL));
        let caret = Rect::from_min_size(
            egui::pos2(rect.left() + CELL, rect.top()),
            Vec2::new(TOOL_CARET, CELL),
        );
        let button = ui
            .scope_builder(egui::UiBuilder::new().max_rect(glyph), |ui| {
                crate::kit::icon_button(ui, "timeline.tool", icon, label, false)
            })
            .inner;
        let caret_hit = ui.interact(
            caret,
            egui::Id::new("timeline.tool.caret"),
            egui::Sense::click(),
        );
        let response = button.union(caret_hit);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        self.remember_icon("timeline.tool", Some(icon.name()), &response);
        icons::tinted(Icon::CaretDown, TOOL_CARET, ink.text()).paint_at(
            ui,
            Rect::from_center_size(caret.center(), Vec2::splat(TOOL_CARET)),
        );
        egui::Popup::menu(&response).show(|ui| {
            for (id, icon, label, tool) in [
                (
                    "timeline.tool.select",
                    Icon::Cursor,
                    Text::ActionTimelineToolSelect,
                    Tool::Select,
                ),
                (
                    "timeline.tool.split",
                    Icon::Scissors,
                    Text::ActionTimelineToolSplit,
                    Tool::Split,
                ),
            ] {
                let mut entry = egui::Button::image_and_text(
                    icons::tinted(icon, 16.0, ink.text()),
                    tr(cx, label),
                )
                .selected(self.tool == tool);
                if let Some(shortcut) = cx.shortcut(id) {
                    entry = entry.shortcut_text(shortcut);
                }
                let response = ui.add(entry);
                self.remember_icon(id, Some(icon.name()), &response);
                if response.clicked() {
                    self.push(Action::Activate(id.to_owned()));
                    ui.close();
                }
            }
        });
    }

    /// The keyframe graph button: enabled while the selection has an easable
    /// segment, and a toggle for the popover below it. The glyph is the keyframe
    /// diamond until the catalog gains a curve icon.
    fn toolbar_graph(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, rect: Rect) {
        let open = self.graph_popover_open();
        let enabled = open || self.graph_available(cx);
        let tooltip = tr(cx, Text::GraphEditor);
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    crate::kit::icon_button(ui, "timeline.graph", Icon::Diamond, tooltip, open)
                })
                .inner
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, open, tooltip)
        });
        self.remember_with(
            "timeline.graph",
            Some(Icon::Diamond.name()),
            Some(tooltip.to_owned()),
            &response,
        );
        if enabled && response.clicked() {
            self.toggle_graph(
                cx,
                egui::pos2(rect.left(), rect.bottom() + theme::space::XS),
            );
        }
    }

    /// A visible toolbar cell whose action is not available yet; its tooltip
    /// carries the reason.
    fn toolbar_unavailable(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        rect: Rect,
        id: &'static str,
        icon: Icon,
        label: Text,
    ) {
        let label = tr(cx, label);
        let tooltip = i18n::format(
            cx.locale,
            Text::ActionUnavailable,
            &[("action", label), ("reason", tr(cx, Text::NotAvailableYet))],
        );
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.add_enabled_ui(false, |ui| {
                    crate::kit::icon_button(ui, id, icon, &tooltip, false)
                })
                .inner
            })
            .inner;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, false, &tooltip));
        self.remember_with(id, Some(icon.name()), Some(tooltip), &response);
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

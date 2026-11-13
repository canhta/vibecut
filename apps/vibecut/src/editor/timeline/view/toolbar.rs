//! Renders timeline actions.
//!
//! Geometry is the measured 32 px strip: 24×24 icon buttons, 4 px edge inset,
//! 12 px between buttons, 1×16 dividers with 8 px either side (221643 y 574-606).

use crate::editor::timeline::{
    Action, Context, Icon, Id, Rect, State, Text, Tick, TimelineLayout, Tool, Vec2, egui, icons,
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

/// The selected clip's id and span when exactly one clip is selected.
fn single_selected_span(cx: &Context<'_>) -> Option<(project::ItemId, Tick, Tick)> {
    let selection = cx.selection();
    if selection.len() != 1 {
        return None;
    }
    let id = selection.iter().copied().next()?;
    let item = cx
        .staged_project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == id)?;
    Some((id, item.start, item.end().ok()?))
}

/// Left-to-right cursor over the toolbar's left group. Only cells that lie wholly
/// between `left` and `right` are drawn or hit; the rest of the row stays laid out
/// so the scroll offset can bring them in.
struct Cursor {
    x: f32,
    top: f32,
    left: f32,
    right: f32,
}
impl Cursor {
    fn cell(&mut self, width: f32) -> Option<Rect> {
        let rect = Rect::from_min_size(egui::pos2(self.x, self.top), Vec2::new(width, CELL));
        self.x += width + GAP;
        (rect.left() >= self.left && rect.right() <= self.right).then_some(rect)
    }
    fn divider(&mut self, ui: &egui::Ui, color: egui::Color32, gap: f32) {
        // The previous cell already added `GAP`; bring the divider `gap` from it.
        let x = self.x - GAP + gap;
        self.x = x + 1.0 + gap;
        if x < self.left || x + 1.0 > self.right {
            return;
        }
        let center = self.top + CELL / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(x + 0.5, center - DIVIDER_HEIGHT / 2.0),
                egui::pos2(x + 0.5, center + DIVIDER_HEIGHT / 2.0),
            ],
            egui::Stroke::new(1.0, color),
        );
    }
}

/// The right group's rects, placed from the right edge inwards.
struct RightGroup {
    /// The leftmost button; the left group must end before it.
    scene: Rect,
    mic: Rect,
    toggles: [Rect; 4],
    /// x of the 1 px divider between the toggles and the zoom buttons.
    divider_x: f32,
    zoom_out: Rect,
    slider: Rect,
    zoom_in: Rect,
}

fn right_group(bar: Rect, top: f32) -> RightGroup {
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
    let divider_x = right;
    right -= 1.0 + DIVIDER_GAP;
    let mut toggles = [Rect::NOTHING; 4];
    for toggle in toggles.iter_mut().rev() {
        *toggle = Rect::from_min_size(egui::pos2(right - CELL, top), Vec2::splat(CELL));
        right = toggle.left() - GAP;
    }
    // The measured right group starts with the mic (221643); the scene
    // button moves one slot along.
    let mic = Rect::from_min_size(egui::pos2(right - CELL - 1.0, top), Vec2::splat(CELL));
    let scene = Rect::from_min_size(egui::pos2(mic.left() - GAP - CELL, top), Vec2::splat(CELL));
    RightGroup {
        scene,
        mic,
        toggles,
        divider_x,
        zoom_out,
        slider,
        zoom_in,
    }
}

/// The left group's scroll offset and the unscrolled right edge of its last cell,
/// both in px, kept per toolbar between frames.
fn left_scroll_id() -> Id {
    Id::new("timeline.toolbar.left_scroll")
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
        let group = right_group(bar, top);
        // The left group may run up to the right group's first button. When the
        // window is too narrow for both, it scrolls under the wheel and cells that
        // would cross that edge stay hidden until scrolled into place.
        let region = Rect::from_min_max(
            egui::pos2(bar.left(), bar.top()),
            egui::pos2(group.scene.left(), bar.bottom()),
        );
        let (scroll, last_extent) = ui
            .data(|data| data.get_temp::<(f32, f32)>(left_scroll_id()))
            .unwrap_or((0.0, 0.0));
        let max_scroll = (last_extent - region.right()).max(0.0);
        let mut scroll = scroll.clamp(0.0, max_scroll);
        if ui.rect_contains_pointer(region) {
            let wheel = ui.input(|input| input.smooth_scroll_delta);
            scroll = (scroll - wheel.x - wheel.y).clamp(0.0, max_scroll);
        }
        let mut left = Cursor {
            x: bar.left() + EDGE - scroll,
            top,
            left: region.left(),
            right: region.right(),
        };
        if let Some(rect) = left.cell(CELL) {
            self.toolbar_button(
                cx,
                ui,
                rect,
                "timeline.add_track",
                Icon::Plus,
                Text::AddTrack,
                None,
            );
        }
        left.divider(ui, divider, DIVIDER_GAP);
        if let Some(tool) = left.cell(CELL + TOOL_CARET) {
            self.toolbar_tool(cx, ui, tool);
        }
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
            if let Some(rect) = left.cell(CELL) {
                self.toolbar_button(cx, ui, rect, id, icon, label, None);
            }
        }
        // Reserved slot after Duplicate, unchanged since the desktop layout was measured.
        // [new] the button is live: it holds the selected video frame at the playhead.
        if let Some(freeze) = left.cell(CELL) {
            self.toolbar_button(
                cx,
                ui,
                freeze,
                "timeline.freeze",
                Icon::Snowflake,
                Text::FreezeFrame,
                None,
            );
        }
        if let Some(delete) = left.cell(CELL) {
            self.toolbar_button(
                cx,
                ui,
                delete,
                "timeline.delete",
                Icon::Trash,
                Text::Delete,
                None,
            );
        }
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
            if let Some(rect) = left.cell(CELL) {
                self.toolbar_button(cx, ui, rect, id, icon, label, None);
            }
        }
        // [proposed] placement: the download glyph is not in the measured strip, so it
        // sits after Flip vertical and before the graph until a capture measures the slot.
        if let Some(download) = left.cell(CELL + TOOL_CARET) {
            self.toolbar_download(cx, ui, download);
        }
        if let Some(graph) = left.cell(CELL) {
            self.toolbar_graph(cx, ui, graph);
        }
        let extent = left.x - GAP + scroll;
        ui.data_mut(|data| data.insert_temp(left_scroll_id(), (scroll, extent)));

        // Right group, placed from the right edge inwards.
        let center = top + CELL / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(group.divider_x - 0.5, center - DIVIDER_HEIGHT / 2.0),
                egui::pos2(group.divider_x - 0.5, center + DIVIDER_HEIGHT / 2.0),
            ],
            egui::Stroke::new(1.0, divider),
        );
        let RightGroup {
            scene,
            mic,
            toggles,
            zoom_out,
            slider,
            zoom_in,
            ..
        } = group;
        self.toolbar_button(
            cx,
            ui,
            mic,
            "voiceover.open",
            Icon::Microphone,
            Text::ActionVoiceoverOpen,
            None,
        );
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

    /// The download button: the clip glyph with a caret. Its menu lists the two downloads
    /// of the selected clip and is disabled unless exactly one clip is selected.
    fn toolbar_download(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, rect: Rect) {
        let ink = Ink::of(cx);
        let clip = single_selected_span(cx);
        let enabled = clip.is_some();
        let label = tr(cx, Text::DownloadClip);
        let glyph = Rect::from_min_size(rect.min, Vec2::splat(CELL));
        let caret = Rect::from_min_size(
            egui::pos2(rect.left() + CELL, rect.top()),
            Vec2::new(TOOL_CARET, CELL),
        );
        let button = ui
            .scope_builder(egui::UiBuilder::new().max_rect(glyph), |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    crate::kit::icon_button(
                        ui,
                        "timeline.download_clip",
                        Icon::DownloadSimple,
                        label,
                        false,
                    )
                })
                .inner
            })
            .inner;
        let caret_hit = ui.interact(
            caret,
            ui.scope_id().with("timeline.download_clip.caret"),
            egui::Sense::click(),
        );
        let response = button.union(caret_hit);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
        self.remember_icon(
            "timeline.download_clip",
            Some(Icon::DownloadSimple.name()),
            &response,
        );
        icons::tinted(Icon::CaretDown, TOOL_CARET, ink.text()).paint_at(
            ui,
            Rect::from_center_size(caret.center(), Vec2::splat(TOOL_CARET)),
        );
        let Some((item, start, end)) = clip else {
            return;
        };
        egui::Popup::menu(&response).show(|ui| {
            for (mode, text) in super::clips::download_entries(cx, start, end) {
                let entry = egui::Button::image_and_text(
                    icons::tinted(Icon::DownloadSimple, 16.0, ink.text()),
                    text,
                );
                if ui.add(entry).clicked() {
                    self.push(Action::DownloadClip { item, mode });
                    ui.close();
                }
            }
        });
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
            ui.scope_id().with("timeline.tool.caret"),
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

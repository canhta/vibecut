//! Renders timeline clips.

use crate::editor::context::visuals::{StripState, StripTile, WaveBar, WaveState};
use crate::editor::media_panel;
use crate::editor::timeline::{
    Action, Align2, Color32, Context, DragMode, FontId, Icon, Id, ItemParams, ItemView,
    LABEL_CHARS, Locale, MIN_ZOOM_FLOOR, Rect, State, Text, Tick, TimelineLayout, Tool, TrackKind,
    Vec2, audio, display_label, egui, icons, theme,
    view::{Ink, tr},
    visuals::{self},
};
use crate::i18n;

impl State {
    pub(in crate::editor::timeline) fn paint_item(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        modifiers: egui::Modifiers,
    ) {
        let x0 = self.x_at(item.start, layout.body.left());
        let x1 = self.x_at(item.end, layout.body.left());
        let full = Rect::from_min_max(
            egui::pos2(x0, item.top + CLIP_INSET),
            egui::pos2(x1, item.top + item.height - CLIP_INSET),
        );
        let visible = full.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let selected = cx.selection().contains(&item.id);
        // Split mode cuts on a click, so the clip takes no drags.
        let split = self.tool == Tool::Split;
        let response = ui.interact(
            visible,
            Id::unique(("timeline.item", item.id.0)),
            if item.locked || split {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        let ink = Ink::of(cx);
        let dark = ink.dark();
        // A source file gone from disk keeps its asset record: the clip draws the
        // missing state in place of strip and waveform and asks for neither, and
        // names its file so the clip keeps a name while the footage is gone.
        // Caption clips only name their source; Replace media serves Video and Audio.
        let missing_asset = matches!(item.kind, TrackKind::Video | TrackKind::Audio)
            .then(|| {
                item.source_asset.and_then(|id| {
                    cx.staged_project()
                        .data()
                        .assets
                        .iter()
                        .find(|asset| asset.id() == id)
                })
            })
            .flatten()
            .filter(|asset| cx.visuals.availability(asset) == media::AssetAvailability::Missing);
        let name = missing_asset.map(media_panel::asset_name);
        let label = name.as_deref().unwrap_or(item.label.as_str());
        let (fill, border, text) = item_palette(item.kind, dark, selected, item.overlay);
        // The relink state recolours the whole clip: body fill, red border unless selected.
        let (fill, border, text) = if missing_asset.is_some() {
            (
                theme::missing_body(dark),
                if selected {
                    border
                } else {
                    theme::missing_border(dark)
                },
                theme::on_missing(dark),
            )
        } else {
            (fill, border, text)
        };
        // Hidden clips stay editable in the timeline at reduced contrast.
        let dim = if item.hidden { 0.45 } else { 1.0 };
        let (fill, border, text) = (
            fill.gamma_multiply(dim),
            border.gamma_multiply(dim),
            text.gamma_multiply(dim),
        );
        let painter = ui.painter().with_clip_rect(visible);
        painter.rect_filled(visible, theme::radius::PANEL, fill);
        let inner = full.shrink(1.0);
        self.remember_with(
            format!("timeline.item.{}", item.id.0),
            None,
            name.clone(),
            &response,
        );
        match item.kind {
            _ if missing_asset.is_some() => {
                self.paint_missing_media(cx, ui, &painter, item, inner, dark);
            }
            TrackKind::Video if item.overlay.is_none() => {
                let (strip, wave) = video_bands(inner, item.main);
                self.paint_clip_visuals(
                    cx,
                    ui,
                    &painter,
                    item,
                    ClipBands {
                        strip: Some(strip),
                        wave,
                        visible,
                        left: layout.body.left(),
                    },
                    dark,
                );
            }
            TrackKind::Audio => {
                self.paint_clip_visuals(
                    cx,
                    ui,
                    &painter,
                    item,
                    ClipBands {
                        strip: None,
                        wave: inner,
                        visible,
                        left: layout.body.left(),
                    },
                    dark,
                );
            }
            _ => {}
        }
        painter.rect_stroke(
            full,
            theme::radius::PANEL,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        if !label.is_empty() {
            // A missing video clip names its file in the top band, as the reference does.
            let band_label =
                missing_asset.is_some() && item.kind == TrackKind::Video && item.overlay.is_none();
            let label_y = if band_label {
                inner.top() + theme::size::CLIP_BAND / 2.0
            } else {
                visible.center().y
            };
            painter.text(
                egui::pos2(visible.left() + 22.0, label_y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(theme::type_size::BODY),
                text,
            );
        }
        self.paint_lane_toggle(cx, ui, item, visible, ink);
        if self.drag.as_ref().is_some_and(|drag| {
            drag.mode == DragMode::Move && drag.origins.iter().any(|origin| origin.item == item.id)
        }) {
            // The dragged clip wears a 3 px white outline (measured 6 px at 2x).
            painter.rect_stroke(
                full,
                theme::radius::PANEL,
                egui::Stroke::new(3.0, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
        if item.kind == TrackKind::Audio {
            self.paint_volume_line(cx, ui, item, inner, ink, layout);
        }
        if !item.locked && !split && visible.width() > 16.0 {
            let width = 8.0_f32.min(visible.width() / 3.0);
            let left = Rect::from_min_max(
                visible.left_top(),
                egui::pos2(visible.left() + width, visible.bottom()),
            );
            let right = Rect::from_min_max(
                egui::pos2(visible.right() - width, visible.top()),
                visible.right_bottom(),
            );
            for (id, rect, mode) in [
                ("trim_start", left, DragMode::TrimStart),
                ("trim_end", right, DragMode::TrimEnd),
            ] {
                let response = ui.interact(
                    rect,
                    Id::unique(("timeline.item.handle", item.id.0, id)),
                    egui::Sense::click_and_drag(),
                );
                self.remember(format!("timeline.item.{}.{id}", item.id.0), &response);
                if selected || response.hovered() {
                    ui.painter().with_clip_rect(rect).line_segment(
                        [rect.center_top(), rect.center_bottom()],
                        egui::Stroke::new(2.0, ink.accent()),
                    );
                }
                if response.clicked() {
                    self.push(Action::Select {
                        item: item.id,
                        modifiers,
                    });
                }
                if response.drag_started() && self.drag.is_none() {
                    self.begin_drag(cx, item.id, mode, ui);
                }
            }
        }
        if item.kind == TrackKind::Audio {
            self.paint_fades(cx, ui, layout, item, inner, visible, ink, modifiers);
        }
        if split {
            // A click cuts at the snapped pointer time when it falls inside the clip;
            // a locked clip or an edge refuses by doing nothing.
            if response.clicked()
                && !item.locked
                && let Some(at) = self.split_cut(cx, ui, layout)
                && item.start < at
                && at < item.end
            {
                self.push(Action::Command(Box::new(engine::Command::SplitAt {
                    item: item.id,
                    at,
                })));
            }
        } else if response.clicked() {
            self.push(Action::Select {
                item: item.id,
                modifiers,
            });
        }
        if !item.locked && self.drag.is_none() && response.drag_started() {
            let press = ui.input(|input| input.pointer.press_origin());
            if item.kind == TrackKind::Audio
                && press
                    .is_some_and(|origin| self.near_volume_line(cx, item, inner, origin, layout))
            {
                self.begin_volume_drag(cx, ui, layout, item, inner);
            } else {
                self.begin_drag(cx, item.id, DragMode::Move, ui);
            }
        }
        if response.secondary_clicked() && !selected {
            self.push(Action::Select {
                item: item.id,
                modifiers: egui::Modifiers::NONE,
            });
        }
        response.context_menu(|ui| self.item_menu(cx, ui, item));
    }

    /// The clip context menu: split variants, duplicate, delete, hide/mute and
    /// source-audio extraction, each dispatching its registered action id.
    fn item_menu(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, item: &ItemView) {
        let paste = format!("timeline.item.{}.paste", item.id.0);
        let enabled = !cx.disabled.contains("edit.paste")
            && cx.selection().len() == 1
            && cx.selection().contains(&item.id);
        if self.menu_action(
            cx,
            ui,
            paste,
            "edit.paste",
            Text::ActionEditPaste,
            Icon::Copy,
            enabled,
        ) {
            self.push(Action::Activate("edit.paste".to_owned()));
            ui.close();
        }
        let editable = !item.locked;
        let at = cx.playhead;
        let splittable = editable && item.start < at && at < item.end;
        let split = format!("timeline.item.{}.split", item.id.0);
        if self.menu_action(
            cx,
            ui,
            split,
            "timeline.split",
            Text::Split,
            Icon::Scissors,
            splittable,
        ) {
            self.push(Action::Activate("timeline.split".to_owned()));
            ui.close();
        }
        let split_left = format!("timeline.item.{}.split_left", item.id.0);
        if self.menu_action(
            cx,
            ui,
            split_left,
            "timeline.split_left",
            Text::SplitLeft,
            Icon::ArrowLeft,
            splittable,
        ) {
            self.push(Action::Activate("timeline.split_left".to_owned()));
            ui.close();
        }
        let split_right = format!("timeline.item.{}.split_right", item.id.0);
        if self.menu_action(
            cx,
            ui,
            split_right,
            "timeline.split_right",
            Text::SplitRight,
            Icon::ArrowsLeftRight,
            splittable,
        ) {
            self.push(Action::Activate("timeline.split_right".to_owned()));
            ui.close();
        }
        let duplicate = format!("timeline.item.{}.duplicate", item.id.0);
        if self.menu_action(
            cx,
            ui,
            duplicate,
            "timeline.duplicate",
            Text::Duplicate,
            Icon::Copy,
            editable,
        ) {
            self.push(Action::Activate("timeline.duplicate".to_owned()));
            ui.close();
        }
        let delete = format!("timeline.item.{}.delete", item.id.0);
        if self.menu_action(
            cx,
            ui,
            delete,
            "timeline.delete",
            Text::Delete,
            Icon::Trash,
            editable,
        ) {
            self.push(Action::Activate("timeline.delete".to_owned()));
            ui.close();
        }
        if item.kind == TrackKind::Video {
            let scenes = format!("timeline.item.{}.split_scene", item.id.0);
            let single = cx.selection().len() == 1 && cx.selection().contains(&item.id);
            if self.menu_action(
                cx,
                ui,
                scenes,
                "timeline.split_scene",
                Text::ActionTimelineSplitScene,
                Icon::MagnifyingGlass,
                editable && single,
            ) {
                self.push(Action::Activate("timeline.split_scene".to_owned()));
                ui.close();
            }
        }
        if item.kind != TrackKind::Audio {
            let (label, icon) = if item.hidden {
                (Text::ShowItem, Icon::Eye)
            } else {
                (Text::HideItem, Icon::EyeSlash)
            };
            let hide = format!("timeline.item.{}.hide", item.id.0);
            if self.menu_action(cx, ui, hide, "timeline.set_hidden", label, icon, editable) {
                self.push(Action::Activate("timeline.set_hidden".to_owned()));
                ui.close();
            }
        }
        if matches!(item.kind, TrackKind::Video | TrackKind::Audio) {
            let muted = item.muted;
            let (label, icon) = if muted {
                (Text::UnmuteItem, Icon::SpeakerHigh)
            } else {
                (Text::MuteItem, Icon::SpeakerSlash)
            };
            let mute = format!("timeline.item.{}.mute", item.id.0);
            if self.menu_action(cx, ui, mute, "timeline.set_muted", label, icon, editable) {
                self.push(Action::Activate("timeline.set_muted".to_owned()));
                ui.close();
            }
        }
        if item.kind == TrackKind::Video {
            let separated = item.source_audio_enabled == Some(false);
            let (label, action) = if separated {
                (Text::RecoverAudio, "timeline.recover_audio")
            } else {
                (Text::ExtractAudio, "timeline.extract_audio")
            };
            let id = format!("timeline.item.{}.audio", item.id.0);
            if self.menu_action(cx, ui, id, action, label, Icon::MusicNotes, editable) {
                self.push(Action::Activate(action.to_owned()));
                ui.close();
            }
        }
        if item.source_asset.is_some() {
            // Relink for a missing source uses the same picker: the new file replaces it.
            let replace = format!("timeline.item.{}.replace", item.id.0);
            if self.menu_action(
                cx,
                ui,
                replace,
                "timeline.replace_media",
                Text::ReplaceMedia,
                Icon::ArrowClockwise,
                editable,
            ) {
                self.push(Action::ReplaceMedia(item.id));
                ui.close();
            }
            let reveal = format!("timeline.item.{}.reveal", item.id.0);
            if self.menu_action(
                cx,
                ui,
                reveal,
                "media.reveal",
                Text::RevealMedia,
                Icon::FolderOpen,
                true,
            ) {
                if let Some(asset) = item.source_asset {
                    self.push(Action::Activate(format!("media.reveal.{}", asset.0)));
                }
                ui.close();
            }
        }
        // Download clip: the clip's own span, with its two downloads in a submenu.
        let ink = Ink::of(cx);
        ui.menu_button(tr(cx, Text::DownloadClip), |ui| {
            for (mode, label) in download_entries(cx, item.start, item.end) {
                let entry = egui::Button::image_and_text(
                    icons::tinted(Icon::DownloadSimple, 16.0, ink.text()),
                    label,
                );
                if ui.add(entry).clicked() {
                    self.push(Action::DownloadClip {
                        item: item.id,
                        mode,
                    });
                    ui.close();
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_action(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        id: String,
        action: &str,
        label: Text,
        icon: Icon,
        enabled: bool,
    ) -> bool {
        let ink = Ink::of(cx);
        let mut button =
            egui::Button::image_and_text(icons::tinted(icon, 16.0, ink.text()), tr(cx, label));
        if let Some(shortcut) = cx.shortcut(action) {
            button = button.shortcut_text(shortcut);
        }
        let response = ui.add_enabled(enabled, button);
        self.remember_icon(id, Some(icon.name()), &response);
        enabled && response.clicked()
    }
}

/// Gap between a lane's edge and its clip: a 76 px video lane holds the 74 px clip.
const CLIP_INSET: f32 = 1.0;

/// The two download entries for a clip span, labelled with the span's `mm:ss` start and end.
pub(super) fn download_entries(
    cx: &Context<'_>,
    start: Tick,
    end: Tick,
) -> [(engine::ClipExportMode, String); 2] {
    let (start, end) = (span_clock(start), span_clock(end));
    let values = [("start", start.as_str()), ("end", end.as_str())];
    [
        (
            engine::ClipExportMode::Clip,
            i18n::format(cx.locale, Text::DownloadSelectedClip, &values),
        ),
        (
            engine::ClipExportMode::Duration,
            i18n::format(cx.locale, Text::DownloadClipDuration, &values),
        ),
    ]
}

/// `mm:ss` of a timeline time, the clock the download entries show.
fn span_clock(at: Tick) -> String {
    let seconds = at.0.max(0) / time::TICKS_PER_SECOND;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

impl State {
    /// A 16 px chevron on the clip toggles its keyframe lanes; a clip at least 20 px tall
    /// shows it, so the 22.5 px clips of the 24 px rows keep it.
    fn paint_lane_toggle(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        item: &ItemView,
        visible: Rect,
        ink: Ink,
    ) {
        if visible.width() < 28.0 || visible.height() < 20.0 {
            return;
        }
        let rect = Rect::from_min_size(
            egui::pos2(visible.left() + 3.0, visible.top() + 3.0),
            Vec2::splat(16.0),
        );
        let response = ui.interact(
            rect,
            Id::unique(("timeline.item.lanes", item.id.0)),
            egui::Sense::click(),
        );
        let open = self.lane_open(item.id);
        let label = tr(
            cx,
            if open {
                Text::HideKeyframes
            } else {
                Text::ShowKeyframes
            },
        );
        self.remember_icon(
            format!("timeline.item.{}.lanes", item.id.0),
            Some(if open {
                Icon::CaretDown.name()
            } else {
                Icon::CaretRight.name()
            }),
            &response,
        );
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, open, label));
        let response = response.on_hover_text(label);
        if response.hovered() || response.has_focus() {
            ui.painter().with_clip_rect(visible).rect_filled(
                rect,
                theme::radius::CONTROL,
                ink.panel(),
            );
        }
        icons::tinted(
            if open {
                Icon::CaretDown
            } else {
                Icon::CaretRight
            },
            12.0,
            ink.text(),
        )
        .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(12.0)));
        if response.clicked() {
            self.push(Action::SetLaneOpen {
                item: item.id,
                open: !open,
            });
        }
    }

    fn paint_volume_line(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        item: &ItemView,
        inner: Rect,
        ink: Ink,
        layout: &TimelineLayout,
    ) {
        let Some(base) = item.volume else {
            return;
        };
        let painter = ui.painter().with_clip_rect(inner);
        let channel = volume_channel(cx, item.id);
        let visible = inner.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let mut line_top = f32::INFINITY;
        let mut line_bottom = f32::NEG_INFINITY;
        let line = theme::accent(ink.dark()).gamma_multiply(0.9);
        let stroke = egui::Stroke::new(1.5, line);
        let step = 8.0;
        let first = ((visible.left() - inner.left()) / step).floor() * step;
        let mut previous: Option<egui::Pos2> = None;
        let mut x = first;
        let mut guard = 0;
        while x <= visible.right() - inner.left() + step && guard < 4096 {
            let px = inner.left() + x;
            let at = self.tick_at(px, layout.body.left());
            let db = channel.map_or(base, |channel| {
                item_db_at(channel, at.saturating_sub(item.start), base)
            });
            let point = egui::pos2(px, inner.top() + audio::line_fraction(db) * inner.height());
            line_top = line_top.min(point.y);
            line_bottom = line_bottom.max(point.y);
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
            x += step;
            guard += 1;
        }
        if let Some(channel) = channel {
            for key in channel.keys() {
                let Some(at) = item.start.checked_add(key.time) else {
                    continue;
                };
                let px = self.x_at(at, layout.body.left());
                if px < visible.left() - 3.0 || px > visible.right() + 3.0 {
                    continue;
                }
                let db = match &key.value {
                    project::ParamValue::Scalar(value) => *value as f32,
                    _ => base,
                };
                let y = inner.top() + audio::line_fraction(db) * inner.height();
                painter.circle_filled(egui::pos2(px, y), 2.5, ink.panel());
                painter.circle_stroke(egui::pos2(px, y), 2.5, stroke);
            }
        }
        // A 12 px band around the line is the drag surface; the rest of the clip
        // still moves, and a plain click falls through to selection.
        let band = Rect::from_min_max(
            egui::pos2(visible.left(), (line_top - 6.0).max(visible.top())),
            egui::pos2(visible.right(), (line_bottom + 6.0).min(visible.bottom())),
        )
        .intersect(visible);
        // Split mode shows the line but takes no drag, so the clip's click cuts.
        let response = ui.interact(
            band,
            Id::unique(("timeline.volume", item.id.0)),
            if self.tool == Tool::Split {
                egui::Sense::hover()
            } else {
                egui::Sense::drag()
            },
        );
        self.remember(format!("timeline.volume.{}", item.id.0), &response);
        if response.drag_started() && self.volume_drag.is_none() {
            self.begin_volume_drag(cx, ui, layout, item, inner);
        }
        // Hover and drag show the exact dB under the pointer.
        let pointer = ui.ctx().input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos())
        });
        let Some(pointer) = pointer else {
            return;
        };
        if !inner.x_range().contains(pointer.x) {
            return;
        }
        let at = self.tick_at(pointer.x, layout.body.left());
        let db = channel.map_or(base, |channel| {
            item_db_at(channel, at.saturating_sub(item.start), base)
        });
        let y = inner.top() + audio::line_fraction(db) * inner.height();
        let dragging = self.volume_drag.is_some_and(|drag| drag.item == item.id);
        if !dragging && (!response.hovered() || (pointer.y - y).abs() > 6.0) {
            return;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        let text = format!("{} dB", i18n::decimal(cx.locale, f64::from(db), 1));
        let anchor = egui::pos2(
            pointer.x.min(inner.right() - 48.0),
            (pointer.y - 18.0).max(inner.top()),
        );
        let galley = painter.layout_no_wrap(
            text,
            FontId::proportional(theme::type_size::BADGE),
            ink.text(),
        );
        let rect = Rect::from_min_size(
            egui::pos2(anchor.x - 4.0, anchor.y - 2.0),
            Vec2::new(galley.size().x + 8.0, galley.size().y + 4.0),
        );
        painter.rect_filled(rect, theme::radius::CONTROL, ink.panel());
        painter.rect_stroke(
            rect,
            theme::radius::CONTROL,
            egui::Stroke::new(1.0, ink.border()),
            egui::StrokeKind::Inside,
        );
        painter.galley(egui::pos2(anchor.x, anchor.y), galley, ink.text());
    }

    /// Whether a press point is close enough to the volume line to grab it.
    fn near_volume_line(
        &self,
        cx: &Context<'_>,
        item: &ItemView,
        inner: Rect,
        point: egui::Pos2,
        layout: &TimelineLayout,
    ) -> bool {
        if !inner.x_range().contains(point.x) {
            return false;
        }
        let Some(base) = item.volume else {
            return false;
        };
        let channel = volume_channel(cx, item.id);
        let at = self.tick_at(point.x, layout.body.left());
        let db = channel.map_or(base, |channel| {
            item_db_at(channel, at.saturating_sub(item.start), base)
        });
        let y = inner.top() + audio::line_fraction(db) * inner.height();
        (point.y - y).abs() <= 6.0
    }
}

/// Fade handle face radius at an audio clip's bottom corner.
const FADE_HANDLE_RADIUS: f32 = 4.0;
/// Pointer target around a fade handle.
const FADE_HANDLE_HIT: f32 = 16.0;
/// Gap between a fade handle and the clip's edge and bottom border.
const FADE_HANDLE_INSET: f32 = 7.0;
/// Longest gap between sampled curve points.
const FADE_CURVE_STEP: f32 = 4.0;
/// Cap on the samples one fade curve spends.
const FADE_CURVE_SAMPLES: usize = 96;

impl State {
    /// Paint an audio clip's two fade handles and their gain curves.
    #[allow(clippy::too_many_arguments)]
    fn paint_fades(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        inner: Rect,
        visible: Rect,
        ink: Ink,
        modifiers: egui::Modifiers,
    ) {
        let Some(fades) = item.fades else {
            return;
        };
        if item.locked || visible.width() < 32.0 || inner.height() < 20.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(visible);
        for edge in [project::FadeEdge::In, project::FadeEdge::Out] {
            let fade = match edge {
                project::FadeEdge::In => fades.fade_in,
                project::FadeEdge::Out => fades.fade_out,
            };
            self.paint_fade_curve(&painter, layout, item, inner, visible, edge, fade, ink);
            let center = match edge {
                project::FadeEdge::In => egui::pos2(
                    inner.left() + FADE_HANDLE_INSET,
                    inner.bottom() - FADE_HANDLE_INSET,
                ),
                project::FadeEdge::Out => egui::pos2(
                    inner.right() - FADE_HANDLE_INSET,
                    inner.bottom() - FADE_HANDLE_INSET,
                ),
            };
            if !visible.contains(center) {
                continue;
            }
            let id = fade_edge_id(edge);
            // Split mode keeps the handle visible but takes no click or drag.
            let response = ui.interact(
                Rect::from_center_size(center, Vec2::splat(FADE_HANDLE_HIT)),
                Id::unique(("timeline.item.fade", item.id.0, id)),
                if self.tool == Tool::Split {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click_and_drag()
                },
            );
            let label = tr(
                cx,
                match edge {
                    project::FadeEdge::In => Text::FadeIn,
                    project::FadeEdge::Out => Text::FadeOut,
                },
            );
            self.remember_with(
                format!("timeline.item.{}.{id}", item.id.0),
                None,
                Some(label.to_owned()),
                &response,
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            if response.hovered()
                || self
                    .fade_drag
                    .is_some_and(|drag| drag.item == item.id && drag.edge == edge)
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            painter.circle_filled(center, FADE_HANDLE_RADIUS, ink.panel());
            painter.circle_stroke(
                center,
                FADE_HANDLE_RADIUS,
                egui::Stroke::new(1.5, ink.accent()),
            );
            if response.clicked() {
                self.push(Action::Select {
                    item: item.id,
                    modifiers,
                });
            }
            if response.drag_started() && self.fade_drag.is_none() {
                self.begin_fade_drag(ui, item, edge);
            }
            self.fade_curve_menu(cx, item, edge, fade, &response);
            let _ = response.on_hover_text(label);
        }
    }

    /// The fade's gain curve, sampled through the project's own curve evaluation.
    #[allow(clippy::too_many_arguments)]
    fn paint_fade_curve(
        &self,
        painter: &egui::Painter,
        layout: &TimelineLayout,
        item: &ItemView,
        inner: Rect,
        visible: Rect,
        edge: project::FadeEdge,
        fade: project::Fade,
        ink: Ink,
    ) {
        let duration = fade.duration;
        let window = fade.window();
        if duration <= Tick::ZERO || window <= Tick::ZERO {
            return;
        }
        let span = (window.seconds() * f64::from(self.zoom)) as f32;
        if !span.is_finite() || span <= 0.0 {
            return;
        }
        let start = self.x_at(item.start, layout.body.left());
        let end = self.x_at(item.end, layout.body.left());
        if start > visible.right() || end < visible.left() {
            return;
        }
        let offset = fade.offset.max(Tick::ZERO).min(duration);
        let first = (offset.seconds() / duration.seconds()).clamp(0.0, 1.0) as f32;
        let steps = ((span / FADE_CURVE_STEP).ceil() as usize).clamp(1, FADE_CURVE_SAMPLES);
        let stroke = egui::Stroke::new(1.5, ink.accent());
        // Keep the gain ramps inside the border so they read apart from it.
        let top = inner.top() + 2.0;
        let bottom = inner.bottom() - 2.0;
        let height = bottom - top;
        let mut previous: Option<egui::Pos2> = None;
        for index in 0..=steps {
            let t = index as f32 / steps as f32;
            let (x, progress) = match edge {
                project::FadeEdge::In => (start + span * t, first + (1.0 - first) * t),
                project::FadeEdge::Out => (end - span + span * t, 1.0 - (1.0 - first) * t),
            };
            let gain = fade.curve.gain(f64::from(progress));
            let point = egui::pos2(x, bottom - gain * height);
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    /// Right-click menu on a fade handle: the edge's curve shape.
    fn fade_curve_menu(
        &mut self,
        cx: &Context<'_>,
        item: &ItemView,
        edge: project::FadeEdge,
        fade: project::Fade,
        response: &egui::Response,
    ) {
        let current = fade.curve;
        let duration = fade.window();
        let edge_id = fade_edge_id(edge);
        response.context_menu(|ui| {
            for curve in [
                project::FadeCurve::EqualPower,
                project::FadeCurve::Linear,
                project::FadeCurve::Exponential,
            ] {
                let response =
                    ui.selectable_label(current == curve, fade_curve_label(cx.locale, curve));
                self.remember(
                    format!(
                        "timeline.item.{}.{edge_id}.curve.{}",
                        item.id.0,
                        fade_curve_id(curve)
                    ),
                    &response,
                );
                if response.clicked() {
                    self.push(Action::Command(Box::new(engine::Command::SetFade {
                        item: item.id,
                        edge,
                        duration,
                        curve,
                    })));
                    ui.close();
                }
            }
        });
    }
}

fn fade_edge_id(edge: project::FadeEdge) -> &'static str {
    match edge {
        project::FadeEdge::In => "fade_in",
        project::FadeEdge::Out => "fade_out",
    }
}
fn fade_curve_id(curve: project::FadeCurve) -> &'static str {
    match curve {
        project::FadeCurve::EqualPower => "equal_power",
        project::FadeCurve::Linear => "linear",
        project::FadeCurve::Exponential => "exponential",
    }
}
fn fade_curve_label(locale: Locale, curve: project::FadeCurve) -> &'static str {
    i18n::text(
        locale,
        match curve {
            project::FadeCurve::EqualPower => Text::FadeSmooth,
            project::FadeCurve::Linear => Text::FadeLinear,
            project::FadeCurve::Exponential => Text::FadeSharp,
        },
    )
}

fn volume_channel<'a>(
    cx: &'a Context<'_>,
    item: crate::editor::timeline::ItemId,
) -> Option<&'a project::Channel> {
    cx.staged_project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| {
            candidate
                .animations
                .get(&project::AnimationPath::from(project::BuiltinPath::Volume))
        })
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
}
fn item_db_at(channel: &project::Channel, time: crate::editor::timeline::Tick, base: f32) -> f32 {
    match channel.value_at(time) {
        project::ParamValue::Scalar(value) => (value as f32).clamp(audio::DB_MIN, audio::DB_MAX),
        _ => base,
    }
}

/// One waveform column is 3 logical px wide, matching the drawn bar pitch.
const BAR_PX: f32 = 3.0;
/// A 16:9 placeholder tile until the first strip frame arrives.
const PLACEHOLDER_ASPECT: f32 = 16.0 / 9.0;

/// The two media bands of one clip and the mapping back to output time.
struct ClipBands {
    strip: Option<Rect>,
    wave: Rect,
    visible: Rect,
    /// Timeline body left edge; clip rectangles start at this origin's tick.
    left: f32,
}

impl State {
    /// Paint the cache-only strip and waveform for one clip. The panel asks the
    /// shared display query for ready tiles/bars and never decodes or keys
    /// media itself; a pending display only draws quiet placeholders.
    fn paint_clip_visuals(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        painter: &egui::Painter,
        item: &ItemView,
        bands: ClipBands,
        dark: bool,
    ) {
        let ClipBands {
            strip,
            wave,
            visible,
            left,
        } = bands;
        let data = cx.staged_project().data();
        let source = data
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item.id);
        let asset = source
            .and_then(|candidate| candidate.source_asset)
            .and_then(|id| data.assets.iter().find(|asset| asset.id() == id));
        let range = time::TimeRange::new(
            self.tick_at(visible.left(), left),
            self.tick_at(visible.right(), left),
        )
        .ok();
        let (Some(source), Some(asset), Some(range)) = (source, asset, range) else {
            if let Some(strip) = strip {
                paint_placeholders(painter, strip, dark);
            }
            paint_flat(painter, wave, theme::waveform(dark).gamma_multiply(0.35));
            return;
        };
        let waveform_ink = if item.kind == TrackKind::Audio {
            theme::audio_waveform(dark)
        } else {
            theme::waveform(dark)
        };
        let tip = visuals::clip_tip(dark);
        if let Some(strip) = strip {
            let state = cx.visuals.strip(
                ui.ctx(),
                asset,
                source,
                range,
                f64::from(self.zoom.max(MIN_ZOOM_FLOOR)),
            );
            match &state {
                StripState::Ready(tiles) => paint_strip_tiles(self, painter, strip, left, tiles),
                StripState::Pending => paint_placeholders(painter, strip, dark),
                StripState::Failed | StripState::Unsupported => {
                    painter.rect_filled(strip, 0.0, theme::field(dark));
                    paint_marker(painter, strip, dark);
                }
            }
            let label = match state {
                StripState::Ready(_) => tr(cx, Text::TimelineThumbnails).to_owned(),
                StripState::Pending => tr(cx, Text::Loading).to_owned(),
                StripState::Failed => tr(cx, Text::VisualFailed).to_owned(),
                StripState::Unsupported => tr(cx, Text::VisualUnsupported).to_owned(),
            };
            self.remember_visual(ui, item, "filmstrip", strip, label);
        }
        let columns =
            ((visible.width() / BAR_PX).ceil() as usize).clamp(1, engine::waveform::MAX_COLUMNS);
        let state = cx.visuals.waveform(asset, source, range, columns);
        let reach = if strip.is_some() {
            0.9
        } else {
            audio_reach(cx.preferences.waveform_size)
        };
        match &state {
            WaveState::Ready(bars) => {
                paint_bars(painter, wave, visible, bars, reach, waveform_ink, tip)
            }
            WaveState::Pending => paint_flat(painter, wave, waveform_ink.gamma_multiply(0.35)),
            WaveState::Failed | WaveState::Unsupported => {
                paint_flat(painter, wave, waveform_ink.gamma_multiply(0.35));
                paint_marker(painter, wave, dark);
            }
        }
        let clipped = matches!(&state, WaveState::Ready(bars) if bars.iter().any(|bar| bar.over));
        let label = match state {
            WaveState::Ready(_) if clipped => format!(
                "{}; {}",
                tr(cx, Text::TimelineWaveform),
                tr(cx, Text::VisualClipping)
            ),
            WaveState::Ready(_) => tr(cx, Text::TimelineWaveform).to_owned(),
            WaveState::Pending => tr(cx, Text::Loading).to_owned(),
            WaveState::Failed => tr(cx, Text::VisualFailed).to_owned(),
            WaveState::Unsupported => tr(cx, Text::VisualUnsupported).to_owned(),
        };
        self.remember_visual(ui, item, "waveform", wave, label);
    }

    /// The relink state, measured on CapCut 9.7.1 (editor.md "Missing media"): the clip body
    /// takes the missing fill, the top and bottom bands the lighter red, and two centred
    /// white lines say the media is lost. There is no glyph. The bands carry the hover and
    /// accessibility name that points at Link media.
    fn paint_missing_media(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        painter: &egui::Painter,
        item: &ItemView,
        inner: Rect,
        dark: bool,
    ) {
        let label = tr(cx, Text::MediaMissing).to_owned();
        let bands = match (item.kind, item.overlay) {
            (TrackKind::Video, None) => {
                let (strip, wave) = video_bands(inner, item.main);
                let top = Rect::from_min_max(
                    inner.left_top(),
                    egui::pos2(inner.right(), strip.top().min(inner.bottom())),
                );
                painter.rect_filled(top, 0.0, theme::missing_band(dark));
                painter.rect_filled(wave, 0.0, theme::missing_band(dark));
                vec![("filmstrip", strip), ("waveform", wave)]
            }
            (TrackKind::Audio, _) => vec![("waveform", inner)],
            _ => vec![("missing", inner)],
        };
        let shown = inner.intersect(painter.clip_rect());
        if shown.width() > 40.0 && shown.height() > 24.0 {
            let ink = theme::on_missing(dark);
            let font = FontId::proportional(theme::type_size::BODY);
            let line = theme::type_size::BODY + 1.0;
            painter.text(
                egui::pos2(shown.center().x, shown.center().y - line / 2.0),
                Align2::CENTER_CENTER,
                tr(cx, Text::MissingClipTitle),
                font.clone(),
                ink,
            );
            painter.text(
                egui::pos2(shown.center().x, shown.center().y + line / 2.0),
                Align2::CENTER_CENTER,
                tr(cx, Text::MissingClipHint),
                font,
                ink,
            );
        }
        for (band, rect) in bands {
            self.remember_visual(ui, item, band, rect, label.clone());
        }
    }

    /// Publish one visual band's state for automation and its hover name.
    fn remember_visual(
        &mut self,
        ui: &egui::Ui,
        item: &ItemView,
        band: &str,
        rect: Rect,
        label: String,
    ) {
        if !rect.is_positive() {
            return;
        }
        let response = ui.interact(
            rect,
            Id::unique(("timeline.item.visual", item.id.0, band)),
            egui::Sense::hover(),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label.as_str()));
        self.remember_with(
            format!("timeline.item.{}.{band}", item.id.0),
            None,
            Some(label.clone()),
            &response,
        );
        let _ = response.on_hover_text(label);
    }
}

/// Frame tiles on their output slices: cached frames or the wait placeholder.
fn paint_strip_tiles(
    state: &State,
    painter: &egui::Painter,
    strip: Rect,
    left: f32,
    tiles: &[StripTile],
) {
    let visible = strip.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    for tile in tiles {
        let rect = Rect::from_min_max(
            egui::pos2(state.x_at(tile.output.start(), left), strip.top()),
            egui::pos2(state.x_at(tile.output.end(), left), strip.bottom()),
        )
        .intersect(visible);
        if !rect.is_positive() {
            continue;
        }
        painter.image(
            tile.texture.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

/// Quiet placeholder frames so the strip still reads as footage.
fn paint_placeholders(painter: &egui::Painter, strip: Rect, dark: bool) {
    let tile = (strip.height() * PLACEHOLDER_ASPECT).max(8.0);
    let hover = theme::hover(dark);
    let visible = strip.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let first = ((visible.left() - strip.left()) / tile).floor();
    let left = visible.left() - (visible.left() - strip.left()).rem_euclid(tile);
    let count = ((visible.right() - left) / tile).ceil() as usize;
    for index in 0..count {
        let x = left + index as f32 * tile;
        let rect = Rect::from_min_size(egui::pos2(x, strip.top()), Vec2::new(tile, strip.height()));
        let shade = if (first + index as f32).rem_euclid(2.0) < 1.0 {
            hover
        } else {
            theme::field(dark)
        };
        painter.rect_filled(rect.shrink(0.5), 0.0, shade);
    }
}

/// A typed 12 px marker for failed or unsupported media.
fn paint_marker(painter: &egui::Painter, area: Rect, dark: bool) {
    if area.width() < 14.0 || area.height() < 12.0 {
        return;
    }
    let rect = Rect::from_min_size(
        egui::pos2(area.left() + 4.0, area.center().y - 6.0),
        Vec2::new(12.0, 12.0),
    );
    painter.rect_filled(rect, 2.0, theme::warning(dark));
    let ink = theme::on_accent(dark);
    let bar = Rect::from_min_size(
        egui::pos2(rect.center().x - 1.0, rect.top() + 2.0),
        Vec2::new(2.0, 5.0),
    );
    painter.rect_filled(bar, 0.0, ink);
    painter.rect_filled(
        Rect::from_min_size(
            egui::pos2(rect.center().x - 1.0, rect.bottom() - 4.0),
            Vec2::splat(2.0),
        ),
        0.0,
        ink,
    );
}

/// A flat centre line while bars are pending or unavailable.
fn paint_flat(painter: &egui::Painter, area: Rect, color: Color32) {
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let y = area.center().y;
    painter.line_segment(
        [
            egui::pos2(visible.left(), y),
            egui::pos2(visible.right(), y),
        ],
        egui::Stroke::new(1.0, color),
    );
}

/// Real cache-only bars mapped through the same display floor as the source.
fn paint_bars(
    painter: &egui::Painter,
    area: Rect,
    visible: Rect,
    bars: &[WaveBar],
    reach: f32,
    color: Color32,
    tip: Color32,
) {
    let clip = area.intersect(painter.clip_rect());
    if !clip.is_positive() || bars.is_empty() || !visible.is_positive() {
        return;
    }
    let step = visible.width() / bars.len() as f32;
    let center = area.center().y;
    let half = area.height() * 0.5 * reach;
    for (index, bar) in bars.iter().enumerate() {
        let x = visible.left() + (index as f32 + 0.5) * step;
        if x < clip.left() - 1.0 || x > clip.right() + 1.0 {
            continue;
        }
        let top = center - half * audio::bar_fraction(bar.max);
        let bottom = center + half * audio::bar_fraction(-bar.min);
        let (top, bottom) = if (bottom - top).abs() < 1.0 {
            (center - 0.5, center + 0.5)
        } else {
            (top, bottom)
        };
        painter.line_segment(
            [
                egui::pos2(x, top.max(area.top())),
                egui::pos2(x, bottom.min(area.bottom())),
            ],
            egui::Stroke::new(1.0, color),
        );
        if bar.over {
            // A >0 dBFS sample extends past full scale: cap both ends.
            painter.line_segment(
                [
                    egui::pos2(x, area.top() + 1.0),
                    egui::pos2(x, (area.top() + 4.0).min(area.bottom())),
                ],
                egui::Stroke::new(1.0, tip),
            );
            painter.line_segment(
                [
                    egui::pos2(x, area.bottom() - 1.0),
                    egui::pos2(x, (area.bottom() - 4.0).max(area.top())),
                ],
                egui::Stroke::new(1.0, tip),
            );
        }
    }
}

pub(in crate::editor::timeline) fn item_label(
    content: &ItemParams,
    locale: Locale,
    dark: bool,
) -> (String, Option<[u8; 4]>) {
    match content {
        ItemParams::Video(_) | ItemParams::Audio(_) => (String::new(), None),
        ItemParams::Text(params) => {
            let value = params.content.as_str();
            if value.is_empty() {
                (i18n::text(locale, Text::TextItem).to_owned(), None)
            } else {
                (display_label(value, LABEL_CHARS), None)
            }
        }
        ItemParams::Caption(_) => (i18n::text(locale, Text::Caption).to_owned(), None),
        ItemParams::Sticker(params) => (display_label(params.value(), LABEL_CHARS), None),
        ItemParams::Effect(_) => (i18n::text(locale, Text::Effects).to_owned(), None),
        ItemParams::Graphic(params) => (
            i18n::text(
                locale,
                match params {
                    project::GraphicParams::Rectangle { .. } => Text::GraphicRectangle,
                    project::GraphicParams::Ellipse { .. } => Text::GraphicEllipse,
                    project::GraphicParams::Polygon { .. } => Text::GraphicPolygon,
                    project::GraphicParams::Star { .. } => Text::GraphicStar,
                },
            )
            .to_owned(),
            Some({
                let [r, g, b, _] = params.appearance().0.0;
                [r, g, b, if dark { 90 } else { 60 }]
            }),
        ),
        ItemParams::Overlay(params) => {
            let [r, g, b, _] = params.color.0;
            let color = if dark { [r, g, b, 90] } else { [r, g, b, 60] };
            (String::new(), Some(color))
        }
    }
}
/// Top band, filmstrip and bottom band of a video clip inside its border
/// (102704): 17 px band; the bottom band is 16 px on the main track and 11 px on
/// an overlay track, where the filmstrip is 20 px. The filmstrip takes the rest,
/// so the Short and Tall row heights stretch it.
fn video_bands(inner: Rect, main: bool) -> (Rect, Rect) {
    let wave_height = if main {
        theme::size::CLIP_WAVEFORM
    } else {
        theme::size::OVERLAY_WAVEFORM
    };
    let strip_top = inner.top() + theme::size::CLIP_BAND;
    let strip = Rect::from_min_size(
        egui::pos2(inner.left(), strip_top),
        Vec2::new(
            inner.width(),
            (inner.height() - theme::size::CLIP_BAND - wave_height).max(0.0),
        ),
    );
    let wave = Rect::from_min_max(
        egui::pos2(inner.left(), strip.bottom()),
        inner.right_bottom(),
    );
    (strip, wave)
}
/// How much of the audio band the bars may fill for a waveform size of 30, 60
/// or 100 percent. The reference lists the sizes but not the resulting bar
/// height, so 0.7 (the 30 % default) to 1.0 is [proposed].
fn audio_reach(percent: u8) -> f32 {
    match percent {
        100 => 1.0,
        60 => 0.85,
        _ => 0.7,
    }
}
/// Dark-theme fill by track kind, measured in 102704.
fn kind_fill(kind: TrackKind) -> Color32 {
    match kind {
        TrackKind::Video => theme::VIDEO_FILL_DARK,
        TrackKind::Audio => theme::AUDIO_CLIP_DARK,
        TrackKind::Text => theme::TEXT_CLIP_DARK,
        TrackKind::Sticker => theme::STICKER_CLIP_DARK,
        TrackKind::Effect => theme::EFFECT_CLIP_DARK,
        TrackKind::Filter => theme::FILTER_CLIP_DARK,
        TrackKind::Adjustment => theme::ADJUSTMENT_CLIP_DARK,
    }
}
pub(in crate::editor::timeline) fn item_palette(
    kind: TrackKind,
    dark: bool,
    selected: bool,
    overlay: Option<[u8; 4]>,
) -> (Color32, Color32, Color32) {
    let text = theme::text(dark);
    let (fill, border) = if kind == TrackKind::Video {
        (theme::video_fill(dark), theme::video_border(dark))
    } else {
        // The reference draws no darker edge on these kinds, so the border is the
        // fill itself [proposed].
        let fill = if dark {
            kind_fill(kind)
        } else {
            theme::light_clip(kind_fill(kind))
        };
        (fill, fill)
    };
    let fill = overlay.map_or(fill, |[r, g, b, a]| {
        Color32::from_rgba_unmultiplied(r, g, b, a)
    });
    if selected {
        (fill, theme::accent(dark), text)
    } else {
        (fill, border, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(color: Color32) -> [u8; 3] {
        [color.r(), color.g(), color.b()]
    }

    #[test]
    fn clip_fills_follow_the_measured_kind_colours() {
        // 102704 (dark theme): one fill per track kind.
        for (index, (kind, expected)) in [
            (TrackKind::Video, [0x20, 0x4C, 0x51]),
            (TrackKind::Audio, [0x18, 0x2F, 0x56]),
            (TrackKind::Text, [0x91, 0x4E, 0x3B]),
            (TrackKind::Sticker, [0xCB, 0x96, 0x3C]),
            (TrackKind::Effect, [0x6E, 0x4C, 0x80]),
            (TrackKind::Filter, [0x46, 0x41, 0x88]),
            (TrackKind::Adjustment, [0xA5, 0x88, 0x41]),
        ]
        .into_iter()
        .enumerate()
        {
            let (fill, _, _) = item_palette(kind, true, false, None);
            assert_eq!(rgb(fill), expected, "fill case {index}");
        }
        let (selected, border, _) = item_palette(TrackKind::Audio, true, true, None);
        assert_eq!(rgb(selected), [0x18, 0x2F, 0x56]);
        assert_eq!(border, theme::accent(true), "selection border");
    }

    #[test]
    fn video_bands_match_the_main_and_overlay_clips() {
        // Inside the 1 px border: main 72 = 17 + 39 + 16, overlay 48 = 17 + 20 + 11.
        let main = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(100.0, 72.0));
        let (strip, wave) = video_bands(main, true);
        assert_eq!(
            (strip.top(), strip.height(), wave.height()),
            (17.0, 39.0, 16.0)
        );
        let overlay = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(100.0, 48.0));
        let (strip, wave) = video_bands(overlay, false);
        assert_eq!(
            (strip.top(), strip.height(), wave.height()),
            (17.0, 20.0, 11.0)
        );
        // A short row keeps the bands inside the clip.
        let short = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(100.0, 20.0));
        let (strip, wave) = video_bands(short, true);
        assert_eq!(strip.height(), 0.0);
        assert!(wave.bottom() <= short.bottom() + 0.001);
    }

    #[test]
    fn waveform_size_widens_the_bars() {
        assert!(audio_reach(30) < audio_reach(60) && audio_reach(60) < audio_reach(100));
        assert_eq!(audio_reach(100), 1.0);
    }
}

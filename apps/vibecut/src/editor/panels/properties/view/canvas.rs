//! Renders the Settings tab's project info and background sections.
use super::Panel;
use crate::i18n;
use crate::{
    editor::properties::{Action, ColorTarget, Id, Text, egui, theme},
    kit,
    services::color_picker::palette,
};
use project::AspectPreset;

/// Integer rates offered by the settings select. A project's exact fractional
/// rate stays selectable as its own current row.
const RATES: [time::FrameRate; 5] = [
    time::FrameRate::FPS_24,
    time::FrameRate::FPS_25,
    time::FrameRate::FPS_30,
    time::FrameRate::FPS_60,
    time::FrameRate::FPS_120,
];

const SWATCHES_PER_ROW: usize = 12;
const SWATCH_SIZE: f32 = 20.0;
const SWATCH_ROW: f32 = 24.0;

impl Panel<'_, '_> {
    pub(super) fn settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("inspector.settings")
            .show(ui, |ui| self.settings_sections(ui));
    }
    fn settings_sections(&mut self, ui: &mut egui::Ui) {
        self.panel_section(
            ui,
            "inspector.section.canvas",
            "inspector.canvas",
            Text::Project,
            true,
            false,
            |panel, ui| {
                panel.ratio_row(ui);
                panel.size_row(ui, true);
                panel.size_row(ui, false);
                panel.frame_rate_row(ui);
                panel.reset_original_row(ui);
            },
        );
        self.panel_section(
            ui,
            "inspector.section.background",
            "inspector.background",
            Text::TextBackground,
            true,
            false,
            |panel, ui| {
                panel.color_row(
                    ui,
                    "inspector.canvas_background",
                    Text::CanvasBackground,
                    ColorTarget::Canvas,
                );
                panel.background_palette(ui);
            },
        );
        self.panel_section(
            ui,
            "inspector.cache",
            "inspector.cache",
            Text::CacheTimeline,
            true,
            false,
            |panel, ui| panel.cache_controls(ui),
        );
    }

    fn cache_controls(&mut self, ui: &mut egui::Ui) {
        use crate::editor::context::{CacheAction as A, CachePhase as P};
        let cache = &self.cx.cache;
        if cache.phase == P::Idle && cache.checked.is_none() {
            self.actions.push(Action::Cache(A::Refresh));
        }
        let busy = matches!(cache.phase, P::Inspecting | P::Clearing | P::Cancelling);
        if busy {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::CacheSize));
        let size = if cache.phase == P::Unavailable
            || cache.report.as_ref().is_some_and(|r| r.root_unavailable)
        {
            self.tr(Text::CacheUnavailable).to_owned()
        } else if let Some(report) = &cache.report {
            // Exact bytes, never budget/RSS, and never an unavailable false zero.
            i18n::format(
                self.cx.locale,
                if report.failed == 0 {
                    Text::CacheBytes
                } else {
                    Text::CacheIncomplete
                },
                &[("bytes", &i18n::number(self.cx.locale, report.disk_bytes))],
            )
        } else if cache.phase == P::Failed {
            self.tr(Text::CacheUnavailable).to_owned()
        } else {
            self.tr(Text::Loading).to_owned()
        };
        let response = value_ui
            .label(size)
            .on_hover_text(self.tr(Text::CacheScope));
        self.remember("inspector.cache.size", &response);
        let (_, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        value_ui.horizontal(|ui| {
            let actions: &[(&str, Text, A)] = if busy {
                &[("inspector.cache.cancel", Text::Cancel, A::Cancel)]
            } else {
                &[
                    ("inspector.cache.refresh", Text::CacheRefresh, A::Refresh),
                    ("inspector.cache.clear", Text::CacheClear, A::Clear),
                    ("inspector.cache.retry", Text::Retry, A::Retry),
                ]
            };
            for &(id, text, action) in actions {
                if action == A::Retry && cache.phase != P::Failed && cache.cause.is_none() {
                    continue;
                }
                let enabled = cache.phase != P::Unavailable && cache.phase != P::Cancelling;
                let glyph = match action {
                    A::Clear => kit::Icon::Trash,
                    A::Cancel => kit::Icon::X,
                    A::Retry => kit::Icon::ArrowCounterClockwise,
                    A::Refresh => kit::Icon::ArrowClockwise,
                };
                let response = ui
                    .add_enabled_ui(enabled, |ui| {
                        kit::icon_button(ui, id, glyph, self.tr(text), false)
                    })
                    .inner;
                self.remember_with_icon(id, Some(glyph.name()), &response);
                if response.clicked() {
                    self.actions.push(Action::Cache(action));
                }
            }
        });
        let status = match cache.phase {
            P::Inspecting => self.tr(Text::CacheInspecting).to_owned(),
            P::Clearing => self.tr(Text::CacheClearing).to_owned(),
            P::Cancelling => self.tr(Text::CacheCancelling).to_owned(),
            P::Unavailable => self.tr(Text::CacheUnavailable).to_owned(),
            _ => cache.checked.map_or_else(String::new, |checked| {
                i18n::format(
                    self.cx.locale,
                    Text::CacheChecked,
                    &[("seconds", &checked.elapsed().as_secs().to_string())],
                )
            }),
        };
        let response = ui.label(egui::RichText::new(status).small().color(self.secondary()));
        self.remember("inspector.cache.status", &response);
        if let Some(report) = &cache.report {
            let summary = i18n::format(
                self.cx.locale,
                Text::CacheResult,
                &[
                    ("removed", &report.removed_bytes.to_string()),
                    ("entries", &report.entries.to_string()),
                    ("pinned", &report.in_use.to_string()),
                    ("failed", &report.failed.to_string()),
                    ("missing", &report.missing.to_string()),
                ],
            );
            let response = ui.label(egui::RichText::new(summary).small().color(self.secondary()));
            self.remember("inspector.cache.result", &response);
        }
        let cause = cache.cause.as_deref().or_else(|| {
            cache
                .report
                .as_ref()
                .and_then(|report| report.cause.as_ref())
        });
        if let Some(cause) = cause {
            let key = if cache
                .report
                .as_ref()
                .is_some_and(|report| report.root_unavailable)
            {
                Text::CacheRestart
            } else {
                match cause {
                    engine::EditError::Cancelled => Text::CacheCancelled,
                    engine::EditError::Runtime(engine::RuntimeError::CacheCorrupt) => {
                        Text::CacheCorrupt
                    }
                    engine::EditError::Runtime(engine::RuntimeError::CacheBusy) => Text::CacheBusy,
                    engine::EditError::Unsupported(_) => Text::CacheUnavailable,
                    _ => Text::CacheFailed,
                }
            };
            let response = ui.label(
                egui::RichText::new(self.tr(key))
                    .small()
                    .color(theme::warning(self.cx.dark)),
            );
            self.remember("inspector.cache.error", &response);
        }
    }

    fn preset_text(&self, preset: AspectPreset) -> &'static str {
        if preset == AspectPreset::Original {
            self.tr(Text::Original)
        } else {
            preset.label()
        }
    }

    fn ratio_row(&mut self, ui: &mut egui::Ui) {
        let data = self.cx.session.project().data();
        let current = data.canvas.preset;
        let original = data.original_canvas_size;
        let label =
            current.map_or_else(|| self.tr(Text::Custom), |preset| self.preset_text(preset));
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::Ratio));
        let response = value_ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = theme::size::FIELD;
                egui::ComboBox::from_id_salt(Id::unique("inspector.canvas.ratio"))
                    .selected_text(label)
                    .width(ui.available_width().min(150.0))
                    .show_ui(ui, |ui| {
                        for preset in AspectPreset::ALL {
                            let enabled = preset != AspectPreset::Original || original.is_some();
                            let response = ui
                                .add_enabled_ui(enabled, |ui| {
                                    ui.selectable_label(
                                        current == Some(preset),
                                        self.preset_text(preset),
                                    )
                                })
                                .inner;
                            self.remember(
                                format!("inspector.canvas.ratio.{}", preset.id()),
                                &response,
                            );
                            if response.clicked()
                                && let Some(size) = preset.resolved(original)
                            {
                                self.actions.push(Action::SetCanvas {
                                    width: size.width,
                                    height: size.height,
                                    preset: Some(preset),
                                });
                                ui.close();
                            }
                        }
                    })
                    .response
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(Text::Ratio))
        });
        self.remember("inspector.canvas.ratio", &response);
    }

    fn size_row(&mut self, ui: &mut egui::Ui, width: bool) {
        let data = self.cx.session.project().data();
        let (value, other, key, label) = if width {
            (
                data.canvas.width,
                data.canvas.height,
                "canvas.width",
                Text::CanvasWidth,
            )
        } else {
            (
                data.canvas.height,
                data.canvas.width,
                "canvas.height",
                Text::CanvasHeight,
            )
        };
        let mut next = self
            .state
            .drafts
            .get(key)
            .cloned()
            .unwrap_or_else(|| value.to_string());
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let id = format!(
            "inspector.canvas.{}",
            if width { "width" } else { "height" }
        );
        let response = kit::value_field(&mut value_ui, &id, &mut next, 120.0, None, self.tr(label));
        self.remember(id, &response);
        if response.has_focus() {
            self.state.focus = Some(key.to_owned());
        }
        if response.changed() {
            self.state.drafts.insert(key.to_owned(), next.clone());
        }
        let editing = response.has_focus() || self.state.focus.as_deref() == Some(key);
        if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(key);
            self.state.focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.state.focus = None;
            let Some(draft) = self.state.drafts.remove(key) else {
                return;
            };
            let Ok(parsed) = draft.trim().parse::<u32>() else {
                self.fail(crate::Error::Invalid("canvas dimension"));
                return;
            };
            let (width, height) = if width {
                (parsed, other)
            } else {
                (other, parsed)
            };
            self.actions.push(Action::SetCanvas {
                width,
                height,
                preset: None,
            });
        }
    }

    fn frame_rate_row(&mut self, ui: &mut egui::Ui) {
        let current = self.cx.session.project().data().canvas.fps;
        let label = i18n::format(
            self.cx.locale,
            Text::FpsValue,
            &[("rate", &current.label())],
        );
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::FrameRateLabel));
        let response =
            value_ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size.y = theme::size::FIELD;
                    egui::ComboBox::from_id_salt(Id::unique("inspector.canvas.fps"))
                        .selected_text(label)
                        .width(ui.available_width().min(150.0))
                        .show_ui(ui, |ui| {
                            for rate in RATES {
                                let text = i18n::format(
                                    self.cx.locale,
                                    Text::FpsValue,
                                    &[("rate", &rate.label())],
                                );
                                let response = ui.selectable_label(current == rate, text);
                                self.remember(
                                    format!("inspector.canvas.fps.{}_{}", rate.num(), rate.den()),
                                    &response,
                                );
                                if response.clicked() {
                                    self.actions.push(Action::SetFps {
                                        num: rate.num(),
                                        den: rate.den(),
                                    });
                                    ui.close();
                                }
                            }
                            if !RATES.contains(&current) {
                                let text = i18n::format(
                                    self.cx.locale,
                                    Text::FpsValue,
                                    &[("rate", &current.label())],
                                );
                                let response = ui.selectable_label(true, text).on_hover_text(
                                    format!("{} / {}", current.num(), current.den()),
                                );
                                self.remember("inspector.canvas.fps.current", &response);
                            }
                        })
                        .response
                })
                .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(Text::FrameRateLabel))
        });
        self.remember("inspector.canvas.fps", &response);
    }

    fn reset_original_row(&mut self, ui: &mut egui::Ui) {
        let original = self.cx.session.project().data().original_canvas_size;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        let label = self.tr(Text::ResetOriginal);
        self.row_label(ui, row, label);
        let response = value_ui
            .add_enabled_ui(original.is_some(), |ui| {
                ui.add(
                    egui::Button::new(label)
                        .fill(theme::field(self.cx.dark))
                        .min_size(egui::vec2(150.0, theme::size::FIELD)),
                )
            })
            .inner;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        self.remember("inspector.canvas.reset_original", &response);
        if response.clicked()
            && let Some(size) = original
        {
            self.actions.push(Action::SetCanvas {
                width: size.width,
                height: size.height,
                preset: Some(AspectPreset::Original),
            });
        }
    }

    fn background_palette(&mut self, ui: &mut egui::Ui) {
        let current = self.cx.session.project().data().canvas.background;
        let dark = self.cx.dark;
        let rows = palette::SOLID.len().div_ceil(SWATCHES_PER_ROW);
        egui::ScrollArea::vertical()
            .id_salt("inspector.canvas.palette")
            .max_height(150.0)
            .show_rows(ui, SWATCH_ROW, rows, |ui, range| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for row in range {
                    ui.horizontal(|ui| {
                        for column in 0..SWATCHES_PER_ROW {
                            let index = row * SWATCHES_PER_ROW + column;
                            let Some(color) = palette::solid(index) else {
                                break;
                            };
                            let [r, g, b, a] = color.0;
                            let (rect, response) = ui.allocate_exact_size(
                                egui::Vec2::splat(SWATCH_SIZE),
                                egui::Sense::click(),
                            );
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                egui::Color32::from_rgba_unmultiplied(r, g, b, a),
                            );
                            if response.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(theme::space::HAIRLINE, theme::hover(dark)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if current == color {
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(theme::space::HAIRLINE, theme::accent(dark)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let response = response.on_hover_text(render::color::hex(color));
                            self.remember(format!("inspector.canvas_swatch.{index}"), &response);
                            if response.clicked() {
                                self.actions.push(Action::SetBackground(color));
                            }
                        }
                    });
                }
            });
    }
}

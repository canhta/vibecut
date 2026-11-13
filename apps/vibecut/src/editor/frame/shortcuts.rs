//! Renders shortcut configuration.

use super::State;
use crate::editor::{
    Align2, Color32, Context, Editor, FontId, Icon, Id, Rect, SHORTCUT_CONTROL_HEIGHT,
    SHORTCUT_ROW_HEIGHT, SHORTCUT_TEXT_SIZE, SHORTCUT_TITLE_SIZE, ShortcutTab, Text, Vec2, actions,
    egui, i18n, icons, theme,
};

/// Modal shortcuts editor over the one action registry and preference store.
/// Rows read effective bindings; grouping and conflict rules all
/// stay owned by their existing modules.
pub(super) fn show(state: &mut State, cx: &Context<'_>, ctx: &egui::Context) -> Vec<Action> {
    let mut actions = Vec::new();
    const ROW_HEIGHT: f32 = SHORTCUT_ROW_HEIGHT;
    const CARD_PAD: f32 = 10.0;
    const CONTENT_MAX: f32 = 500.0;
    let dark = cx.dark;
    let platform = actions::Platform::current();
    let width = 720.0_f32.min(ctx.content_rect().width() - 48.0).max(420.0);
    let specs: Vec<&'static actions::ActionSpec> = actions::actions()
        .iter()
        .filter(|spec| spec.bindable() && state.shortcut_tab.categories().contains(&spec.category))
        .collect();
    let mut specs = specs;
    specs.sort_by_key(|spec| {
        state
            .shortcut_tab
            .categories()
            .iter()
            .position(|category| *category == spec.category)
            .unwrap_or(usize::MAX)
    });
    let split = specs.len().div_ceil(2);
    let rows_height = split.max(specs.len() - split) as f32 * ROW_HEIGHT;
    let content_height = rows_height.clamp(96.0, CONTENT_MAX);
    let mut close_requested = false;
    let output = egui::Modal::new(Id::unique("shortcuts.dialog"))
        .frame(crate::kit::dialog_frame(ctx).inner_margin(egui::Margin::ZERO))
        .show(ctx, |ui| {
            ui.set_width(width);
            let (title_rect, _) =
                ui.allocate_exact_size(Vec2::new(width, 42.0), egui::Sense::hover());
            ui.painter().rect_filled(
                title_rect,
                egui::CornerRadius {
                    nw: 12,
                    ne: 12,
                    sw: 0,
                    se: 0,
                },
                theme::window(dark),
            );
            ui.painter().text(
                title_rect.center(),
                Align2::CENTER_CENTER,
                i18n::text(cx.locale, Text::ShortcutsTitle),
                FontId::proportional(SHORTCUT_TITLE_SIZE),
                theme::text(cx.dark),
            );
            let close = ui
                .put(
                    Rect::from_center_size(
                        egui::pos2(title_rect.right() - 24.0, title_rect.center().y),
                        Vec2::splat(theme::size::CONTROL),
                    ),
                    egui::Button::image(icons::tinted(Icon::X, 16.0, theme::secondary(cx.dark)))
                        .frame(false),
                )
                .on_hover_text(i18n::text(cx.locale, Text::ShortcutsClose));
            state.widgets.remember("shortcuts.close", &close);
            if close.clicked() {
                close_requested = true;
            }
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: 16,
                    right: 16,
                    top: 10,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(width - 32.0);
                    ui.horizontal(|ui| {
                        for tab in ShortcutTab::ALL.iter().copied() {
                            shortcut_tab_button(state, cx, ui, tab, dark);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            shortcut_preset(state, cx, ui)
                        });
                    });
                    ui.add_space(theme::space::LG);
                    egui::Frame::NONE
                        .fill(theme::window(dark))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::same(CARD_PAD as i8))
                        .show(ui, |ui| {
                            ui.set_width(width - 32.0 - 2.0 * CARD_PAD);
                            ui.spacing_mut().scroll.floating = false;
                            ui.spacing_mut().scroll.bar_width = 8.0;
                            let mut scroll_style = ui.style().as_ref().clone();
                            scroll_style.visuals.widgets.inactive.fg_stroke.color =
                                theme::secondary(dark);
                            scroll_style.visuals.widgets.hovered.fg_stroke.color =
                                theme::text(dark);
                            ui.set_style(scroll_style);
                            egui::ScrollArea::vertical()
                                .id_salt("shortcuts.scroll")
                                .max_height(content_height)
                                .auto_shrink([false, true])
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                                )
                                .show(ui, |ui| {
                                    let columns = ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        let gap = 14.0;
                                        let column_width = (ui.available_width() - gap) / 2.0;
                                        for column in [0, 1] {
                                            let start = if column == 0 { 0 } else { split };
                                            let end = if column == 0 { split } else { specs.len() };
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(column_width, rows_height),
                                                egui::Layout::top_down(egui::Align::Min),
                                                |ui| {
                                                    ui.spacing_mut().item_spacing.y = 0.0;
                                                    ui.set_width(column_width);
                                                    for spec in specs
                                                        .iter()
                                                        .skip(start)
                                                        .take(end.saturating_sub(start))
                                                        .copied()
                                                    {
                                                        shortcut_row(
                                                            state,
                                                            cx,
                                                            &mut actions,
                                                            ui,
                                                            spec,
                                                            platform,
                                                            dark,
                                                        );
                                                    }
                                                },
                                            );
                                            if column == 0 {
                                                ui.add_space(gap);
                                            }
                                        }
                                    });
                                    let rect = columns.response.rect;
                                    ui.painter().line_segment(
                                        [
                                            egui::pos2(rect.center().x, rect.top()),
                                            egui::pos2(rect.center().x, rect.bottom()),
                                        ],
                                        egui::Stroke::new(1.0, theme::separator(dark)),
                                    );
                                });
                        });
                    if let Some(error) = state.shortcut_error.clone() {
                        ui.add_space(theme::space::MD);
                        let response = ui.add(
                            egui::Label::new(
                                egui::RichText::new(error)
                                    .size(theme::type_size::BODY)
                                    .color(theme::error_text(dark)),
                            )
                            .wrap(),
                        );
                        state.widgets.remember("shortcuts.error", &response);
                    }
                    ui.add_space(theme::space::ROW_INSET);
                    ui.horizontal(|ui| {
                        let can_reset = !cx.keybindings.bindings.is_empty()
                            || !cx.keybindings.is_customized.is_empty();
                        let reset = ui.add_enabled(
                            can_reset,
                            egui::Button::new(i18n::text(cx.locale, Text::ShortcutsResetAll))
                                .min_size(Vec2::new(82.0, SHORTCUT_CONTROL_HEIGHT)),
                        );
                        state.widgets.remember("shortcuts.reset_all", &reset);
                        if reset.clicked() {
                            actions.push(Action::ResetKeybindings { action: None });
                        }
                        let import = ui.add_enabled(
                            false,
                            egui::Button::new(i18n::text(cx.locale, Text::ShortcutsImport))
                                .min_size(Vec2::new(104.0, SHORTCUT_CONTROL_HEIGHT)),
                        );
                        state.widgets.remember("shortcuts.import", &import);
                        let export = ui.add_enabled(
                            false,
                            egui::Button::new(i18n::text(cx.locale, Text::ShortcutsExport))
                                .min_size(Vec2::new(104.0, SHORTCUT_CONTROL_HEIGHT)),
                        );
                        state.widgets.remember("shortcuts.export", &export);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let cancel = ui.add(
                                egui::Button::new(i18n::text(cx.locale, Text::ShortcutsCancel))
                                    .min_size(Vec2::new(84.0, SHORTCUT_CONTROL_HEIGHT)),
                            );
                            state.widgets.remember("shortcuts.cancel", &cancel);
                            if cancel.clicked() {
                                close_requested = true;
                            }
                            let save = ui.add_enabled(
                                false,
                                egui::Button::new(
                                    egui::RichText::new(i18n::text(cx.locale, Text::ShortcutsSave))
                                        .color(theme::on_accent(dark)),
                                )
                                .fill(theme::accent(dark).gamma_multiply(0.28))
                                .min_size(Vec2::new(84.0, SHORTCUT_CONTROL_HEIGHT)),
                            );
                            state.widgets.remember("shortcuts.save", &save);
                        });
                    });
                });
        });
    state.widgets.remember("shortcuts.dialog", &output.response);
    if close_requested || output.backdrop_response.clicked() {
        state.close_shortcuts();
    }
    actions
}
fn shortcut_tab_button(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    tab: ShortcutTab,
    dark: bool,
) {
    let selected = state.shortcut_tab == tab;
    let label = i18n::text(cx.locale, tab.label());
    let font = FontId::proportional(SHORTCUT_TEXT_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), theme::text(cx.dark));
    let width = (galley.size().x + 28.0).max(72.0);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, SHORTCUT_CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    let fill = if selected {
        theme::hover(dark)
    } else if response.hovered() {
        theme::hover(dark).gamma_multiply(0.55)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::radius::CONTROL, fill);
    let color = if selected || response.hovered() {
        theme::text(cx.dark)
    } else {
        theme::secondary(cx.dark)
    };
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    state
        .widgets
        .remember(format!("shortcuts.tab.{}", tab.id()), &response);
    if response.clicked() {
        state.set_shortcut_tab(tab);
    }
}
fn shortcut_preset(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) {
    let label = i18n::text(cx.locale, Text::ShortcutsProfile);
    let mut selected = 0u8;
    let response = egui::ComboBox::from_id_salt(Id::unique("shortcuts.preset"))
        .selected_text(label)
        .width(180.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut selected, 0, label);
        })
        .response
        .on_hover_text(label);
    state.widgets.remember("shortcuts.preset", &response);
}
/// One action row: label, platform glyphs and a per-action reset once the
/// action owns an override. Clicking the row records the next chord.
fn shortcut_row(
    state: &mut State,
    cx: &Context<'_>,
    actions: &mut Vec<Action>,
    ui: &mut egui::Ui,
    spec: &'static actions::ActionSpec,
    platform: actions::Platform,
    dark: bool,
) {
    const HEIGHT: f32 = SHORTCUT_ROW_HEIGHT;
    const RESET: f32 = 22.0;
    let width = ui.available_width().max(180.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), egui::Sense::hover());
    let reset_rect = Rect::from_center_size(
        egui::pos2(rect.right() - RESET / 2.0 - 2.0, rect.center().y),
        Vec2::splat(RESET),
    );
    let row_rect = Rect::from_min_max(rect.min, egui::pos2(reset_rect.left() - 4.0, rect.bottom()));
    let row = ui.interact(
        row_rect,
        ui.make_persistent_id(("shortcuts.row", spec.id)),
        egui::Sense::click(),
    );
    let recording = state.shortcut_recording == Some(spec.id);
    let customized = cx.keybindings.is_customized.contains(spec.id);
    let chords = cx.keybindings.effective_chords(spec);
    let key_color = if recording {
        theme::accent(cx.dark)
    } else {
        theme::secondary(cx.dark)
    };
    if recording {
        ui.painter().rect_filled(
            row_rect,
            theme::radius::CONTROL,
            theme::accent(cx.dark).gamma_multiply(0.18),
        );
    } else if row.hovered() {
        ui.painter()
            .rect_filled(row_rect, theme::radius::CONTROL, theme::hover(dark));
    }
    if customized && !recording {
        ui.painter().circle_filled(
            egui::pos2(row_rect.left() + 5.0, row_rect.center().y),
            3.0,
            theme::accent(cx.dark),
        );
    }
    let key_right = reset_rect.left() - 8.0;
    let key_font = FontId::proportional(SHORTCUT_TEXT_SIZE);
    let key_width = if recording {
        let text = i18n::text(cx.locale, Text::ShortcutsPressKey);
        let width = ui
            .painter()
            .layout_no_wrap(text.to_owned(), key_font.clone(), key_color)
            .size()
            .x;
        ui.painter().text(
            egui::pos2(key_right, row_rect.center().y),
            Align2::RIGHT_CENTER,
            text,
            key_font,
            key_color,
        );
        width
    } else if chords.is_empty() {
        let text = i18n::text(cx.locale, Text::ShortcutsUnbound);
        let width = ui
            .painter()
            .layout_no_wrap(text.to_owned(), key_font.clone(), key_color)
            .size()
            .x;
        ui.painter().text(
            egui::pos2(key_right, row_rect.center().y),
            Align2::RIGHT_CENTER,
            text,
            key_font,
            key_color,
        );
        width
    } else {
        paint_shortcut_chords(ui, key_right, row_rect.center().y, &chords, platform, dark)
    };
    // Long labels wrap inside the row instead of running into the keys.
    let label_max = (key_right - key_width - (row_rect.left() + 16.0) - 8.0).max(60.0);
    let label = ui.painter().layout(
        i18n::text(cx.locale, spec.label).to_owned(),
        FontId::proportional(SHORTCUT_TEXT_SIZE),
        theme::text(cx.dark),
        label_max,
    );
    ui.painter().galley(
        egui::pos2(
            row_rect.left() + 16.0,
            row_rect.center().y - label.size().y * 0.5,
        ),
        label,
        theme::text(cx.dark),
    );
    let key_tooltip = if chords.is_empty() {
        i18n::text(cx.locale, Text::ShortcutsUnbound).to_owned()
    } else {
        chords
            .iter()
            .filter_map(|text| actions::Chord::parse(text).ok())
            .map(|chord| chord.display(platform))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let row_id = format!("shortcuts.row.{}", spec.id);
    state.widgets.remember(&row_id, &row);
    if let Some(record) = state.widgets.get_mut(&row_id) {
        record.tooltip = Some(key_tooltip);
    }
    if row.clicked() {
        state.shortcut_recording = Some(spec.id);
        state.shortcut_error = None;
    }
    let can_reset = cx.keybindings.bindings.contains_key(spec.id);
    if can_reset {
        let image = icons::tinted(Icon::ArrowCounterClockwise, 14.0, theme::secondary(cx.dark));
        let reset = ui.put(
            reset_rect,
            egui::Button::image(image)
                .frame(false)
                .min_size(Vec2::splat(RESET)),
        );
        let reset = reset.on_hover_text(i18n::text(cx.locale, Text::ShortcutsReset));
        state
            .widgets
            .remember(format!("shortcuts.reset.{}", spec.id), &reset);
        if reset.clicked() {
            actions.push(Action::ResetKeybindings {
                action: Some(spec.id),
            });
        }
    }
}
fn paint_shortcut_chords(
    ui: &egui::Ui,
    right: f32,
    center_y: f32,
    chords: &[String],
    platform: actions::Platform,
    dark: bool,
) -> f32 {
    let font = FontId::proportional(SHORTCUT_TEXT_SIZE);
    let mut groups: Vec<Vec<(String, bool, f32)>> = Vec::with_capacity(chords.len());
    for text in chords {
        let Ok(chord) = actions::Chord::parse(text) else {
            continue;
        };
        let mut group = Vec::new();
        for token in chord.parts(platform) {
            let chip = token.chars().count() > 1;
            let width = ui
                .painter()
                .layout_no_wrap(token.clone(), font.clone(), theme::text(dark))
                .size()
                .x;
            group.push((token, chip, width));
        }
        groups.push(group);
    }
    let token_gap = 4.0;
    let chord_gap = 14.0;
    let total = groups
        .iter()
        .map(|group| {
            group.iter().map(|(_, _, width)| *width).sum::<f32>()
                + token_gap * group.len().saturating_sub(1) as f32
        })
        .sum::<f32>()
        + chord_gap * groups.len().saturating_sub(1) as f32;
    let mut x = right - total;
    for (index, group) in groups.iter().enumerate() {
        for (token_index, (token, _, width)) in group.iter().enumerate() {
            ui.painter().text(
                egui::pos2(x + width / 2.0, center_y),
                Align2::CENTER_CENTER,
                token,
                font.clone(),
                theme::text(dark),
            );
            x += width;
            if token_index + 1 < group.len() {
                x += token_gap;
            }
        }
        if index + 1 < groups.len() {
            x += chord_gap;
        }
    }
    total
}
/// What the frame must do after the shortcuts dialog painted.
pub(super) enum Action {
    /// Reset one action's chords, or every override when `action` is `None`.
    ResetKeybindings { action: Option<&'static str> },
}

impl Editor {
    /// Paint the shortcuts dialog through the frame state it owns.
    pub(in crate::editor) fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.frame.shortcuts_open {
            return;
        }
        let mut frame = std::mem::take(&mut self.frame);
        let cx = self.context();
        let actions = show(&mut frame, &cx, ctx);
        drop(cx);
        self.frame = frame;
        for action in actions {
            match action {
                Action::ResetKeybindings { action } => {
                    let result = match action {
                        Some(id) => self.reset_keybinding(id),
                        None => self.reset_keybindings(),
                    };
                    self.frame.shortcut_error = match result {
                        Ok(()) => None,
                        Err(error) => Some(self.shortcut_error_message(&error)),
                    };
                }
            }
        }
    }
}

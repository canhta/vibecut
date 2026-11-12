//! Renders the assets surface from a read-only projection.
use crate::i18n;
mod browser;
mod deletion;
mod effects;
mod feedback;
mod music;
mod sounds;
mod transition;

pub(in crate::editor) use deletion::show as delete_dialog;
pub(in crate::editor) use feedback::{drop_overlay, import_overlay, toast};

use super::{
    Action, Align2, AssetRow, AssetState, AssetsTab, AudioSection, Color32, EffectRow, FontId,
    Icon, Id, MediaKind, MediaSortKey, MediaSortOrder, MediaView, Model, Recorded, Rect, Row,
    State, Text, Vec2, asset_state_color, display_short_time, egui, icons, state, theme,
};
use state::Filter;

pub(in crate::editor) fn show(state: &mut State, cx: &Model, ui: &mut egui::Ui) -> Vec<Action> {
    let mut view = View {
        state,
        cx,
        actions: Vec::new(),
    };
    let rect = ui.max_rect();
    let strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), theme::size::TAB_STRIP));
    ui.painter()
        .rect_filled(strip, 0.0, theme::strip(cx.preferences.dark));
    ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
        egui::ScrollArea::horizontal()
            .id_salt("assets.tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let compact = rect.width() < 300.0;
                    for tab in [
                        AssetsTab::Media,
                        AssetsTab::Sounds,
                        AssetsTab::Text,
                        AssetsTab::Stickers,
                        AssetsTab::Effects,
                        AssetsTab::Transitions,
                        AssetsTab::Captions,
                        AssetsTab::Adjustment,
                    ] {
                        let id = format!("assets.tab.{}", tab.id());
                        let label = view.tr(if tab == AssetsTab::Sounds {
                            Text::Audio
                        } else {
                            tab.label()
                        });
                        let response = if compact {
                            ui.allocate_ui_with_layout(
                                Vec2::new(rect.width() / 8.0, 42.0),
                                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                |ui| {
                                    crate::kit::icon_button(
                                        ui,
                                        &id,
                                        tab.icon(),
                                        label,
                                        tab == cx.preferences.assets_tab,
                                    )
                                },
                            )
                            .inner
                        } else {
                            crate::kit::icon_tab(
                                ui,
                                &id,
                                tab.icon(),
                                label,
                                tab == cx.preferences.assets_tab,
                                if cx.preferences.locale == i18n::Locale::En {
                                    Some(if tab == AssetsTab::Media {
                                        theme::size::TAB_MIN
                                    } else if tab == AssetsTab::Transitions {
                                        // Wider label pitch in the measured feature strip.
                                        61.0
                                    } else {
                                        theme::size::FEATURE_TAB_PITCH
                                    })
                                } else {
                                    None
                                },
                            )
                        };
                        view.remember(id, &response, Some(tab.icon()));
                        if response.clicked() {
                            view.actions.push(Action::SelectTab(tab));
                        }
                    }
                });
            });
    });
    let body = Rect::from_min_max(egui::pos2(rect.left(), strip.bottom()), rect.max);
    if cx.preferences.assets_tab == AssetsTab::Captions {
        view.actions.push(Action::Captions(body.shrink(12.0)));
        return view.actions;
    }
    let nav = Rect::from_min_size(
        body.min,
        Vec2::new(
            if rect.width() < 420.0 {
                0.0
            } else {
                theme::size::SUBNAV.min(body.width())
            },
            body.height(),
        ),
    );
    ui.painter().line_segment(
        [nav.right_top(), nav.right_bottom()],
        egui::Stroke::new(1.0, theme::separator(cx.preferences.dark)),
    );
    let active = match cx.preferences.assets_tab {
        AssetsTab::Media => Text::Import,
        AssetsTab::Sounds => Text::Audio,
        AssetsTab::Text => Text::AssetsAddText,
        AssetsTab::Stickers => Text::GraphicShapes,
        AssetsTab::Effects => Text::Effects,
        tab => tab.label(),
    };
    if nav.width() > 0.0 {
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(nav.shrink2(Vec2::new(8.0, 15.0))),
            |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                if cx.preferences.assets_tab == AssetsTab::Effects {
                    view.effects_nav(ui);
                    return;
                }
                if cx.preferences.assets_tab == AssetsTab::Transitions {
                    view.transition_nav(ui);
                    return;
                }
                if cx.preferences.assets_tab == AssetsTab::Sounds {
                    view.audio_nav(ui);
                    return;
                }
                let label = view.tr(active);
                let response = ui.add_sized(
                    [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
                    egui::Button::new(
                        egui::RichText::new(label)
                            .font(theme::label_font())
                            .size(theme::type_size::SMALL_LABEL)
                            .color(if cx.preferences.assets_tab == AssetsTab::Text {
                                theme::text(cx.preferences.dark)
                            } else {
                                theme::accent(cx.preferences.dark)
                            }),
                    )
                    .fill(theme::hover(cx.preferences.dark))
                    .stroke(egui::Stroke::NONE)
                    .right_text(""),
                );
                if cx.preferences.assets_tab == AssetsTab::Text {
                    view.remember("text.insert", &response, None);
                    if response.clicked() {
                        view.actions.push(Action::Command("text.insert".into()));
                    }
                } else {
                    view.remember("assets.subnav", &response, None);
                }
                if cx.preferences.assets_tab == AssetsTab::Media {
                    let response = ui.add_sized(
                        [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
                        egui::Button::new(
                            egui::RichText::new(view.tr(Text::Media))
                                .font(theme::label_font())
                                .size(theme::type_size::SMALL_LABEL)
                                .color(theme::accent(cx.preferences.dark)),
                        )
                        .fill(theme::hover(cx.preferences.dark))
                        .stroke(egui::Stroke::NONE)
                        .right_text(""),
                    );
                    view.remember("assets.subnav.media", &response, None);
                }
            },
        );
    }
    let content = Rect::from_min_max(
        egui::pos2(
            nav.right() + if nav.width() > 0.0 { 13.0 } else { 12.0 },
            body.top() + 12.0,
        ),
        body.max - Vec2::new(12.0, 12.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
        ui.set_clip_rect(content.intersect(ui.clip_rect()));
        match cx.preferences.assets_tab {
            AssetsTab::Media if cx.rows.is_empty() && !cx.importing && !cx.failed => {
                view.empty_media(ui)
            }
            AssetsTab::Media => view.browser(ui),
            AssetsTab::Sounds => {
                if view.state.audio_section == AudioSection::Music && nav.width() > 0.0 {
                    view.music(ui)
                } else {
                    view.sounds(ui)
                }
            }
            AssetsTab::Effects => view.effects(ui),
            AssetsTab::Transitions => view.transitions(ui),
            AssetsTab::Stickers => view.shapes(ui),
            AssetsTab::Adjustment => view.adjustment(ui),
            AssetsTab::Text => {
                if nav.width() == 0.0
                    && view.icon(ui, "text.insert", Icon::TextT, Text::AssetsAddText, true)
                {
                    view.actions.push(Action::Command("text.insert".into()));
                }
                view.empty(ui, Icon::TextT, Text::AssetsNoStyles);
            }
            tab => {
                let _ = tab;
                view.empty(ui, Icon::FolderOpen, Text::AssetsUnavailable);
            }
        }
    });
    view.actions
}

struct View<'a> {
    state: &'a mut State,
    cx: &'a Model,
    actions: Vec<Action>,
}
impl View<'_> {
    fn shapes(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (index, (id, label)) in [
                ("graphic.insert.rectangle", Text::GraphicRectangle),
                ("graphic.insert.ellipse", Text::GraphicEllipse),
                ("graphic.insert.polygon", Text::GraphicPolygon),
                ("graphic.insert.star", Text::GraphicStar),
            ]
            .into_iter()
            .enumerate()
            {
                ui.vertical(|ui| {
                    let (tile, response) =
                        ui.allocate_exact_size(Vec2::new(88., 88.), egui::Sense::hover());
                    let shape = tile.shrink(20.);
                    if let Some(texture) =
                        self.cx.graphic_previews.get(index).and_then(Option::as_ref)
                    {
                        ui.painter().image(
                            texture.id(),
                            shape,
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                            Color32::WHITE,
                        );
                    } else {
                        ui.painter().text(
                            shape.center(),
                            Align2::CENTER_CENTER,
                            self.tr(Text::AssetsUnavailable),
                            theme::label_font(),
                            theme::text(self.cx.preferences.dark),
                        );
                    }
                    let _ = response;
                    ui.label(
                        egui::RichText::new(self.tr(label))
                            .font(theme::label_font())
                            .color(theme::text(self.cx.preferences.dark)),
                    );
                    if self.icon(ui, id, Icon::Plus, label, self.cx.graphic_insertion_ready) {
                        self.actions.push(Action::Command(id.into()));
                    }
                });
            }
        });
    }

    fn adjustment(&mut self, ui: &mut egui::Ui) {
        let presets = self.cx.preferences.adjustment_presets.clone();
        if presets.is_empty() {
            self.empty(ui, Icon::SlidersHorizontal, Text::AdjustmentPresetsEmpty);
            return;
        }
        for (index, preset) in presets.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&preset.name)
                        .font(theme::label_font())
                        .size(theme::type_size::BODY)
                        .color(theme::text(self.cx.preferences.dark)),
                );
                if self.icon(
                    ui,
                    &format!("assets.adjustment.{index}.apply"),
                    Icon::Plus,
                    Text::CropApply,
                    true,
                ) {
                    self.actions.push(Action::ApplyAdjustment(index));
                }
                if self.icon(
                    ui,
                    &format!("assets.adjustment.{index}.delete"),
                    Icon::Trash,
                    Text::Delete,
                    true,
                ) {
                    self.actions.push(Action::DeleteAdjustment(index));
                }
            });
            ui.add_space(theme::space::SM);
        }
    }

    fn empty_media(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let drop = Rect::from_min_size(
            area.min,
            Vec2::new(area.width(), (area.height() - 121.0).max(24.0)),
        );
        ui.painter().rect_filled(
            drop,
            theme::radius::UPLOAD,
            theme::field(self.cx.preferences.dark),
        );
        let response = ui
            .interact(drop, Id::unique("file.import"), egui::Sense::click())
            .on_hover_text(self.tr(Text::Import));
        crate::kit::dashed_outline(ui, drop, theme::radius::UPLOAD, response.has_focus());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, self.tr(Text::Import))
        });
        self.remember("file.import", &response, Some(Icon::Plus));
        icons::tinted(Icon::Plus, 16.0, theme::accent(self.cx.preferences.dark)).paint_at(
            ui,
            Rect::from_center_size(drop.center() - Vec2::new(26.0, 12.0), Vec2::splat(16.0)),
        );
        let mut title_font = theme::label_font();
        title_font.size = theme::type_size::CHROME;
        ui.painter().text(
            drop.center() + Vec2::new(14.0, -13.0),
            Align2::CENTER_CENTER,
            self.tr(Text::Import),
            title_font,
            theme::text(self.cx.preferences.dark),
        );
        if response.clicked() {
            self.actions.push(Action::Command("file.import".into()));
        }
        let text = Rect::from_min_size(
            egui::pos2(area.left(), drop.bottom() + 41.0),
            Vec2::new(((area.width() - 16.0) / 3.0).max(24.0), 80.0),
        );
        let response = ui
            .interact(text, Id::unique("text.insert"), egui::Sense::click())
            .on_hover_text(self.tr(Text::TextItem));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, self.tr(Text::TextItem))
        });
        self.remember("text.insert", &response, Some(Icon::TextT));
        ui.painter().rect_filled(
            text,
            theme::radius::UPLOAD,
            theme::field(self.cx.preferences.dark),
        );
        icons::tinted(Icon::TextT, 24.0, theme::text(self.cx.preferences.dark))
            .paint_at(ui, Rect::from_center_size(text.center(), Vec2::splat(24.0)));
        if response.clicked() {
            self.actions.push(Action::Command("text.insert".into()));
        }
    }
    fn tr(&self, text: Text) -> &'static str {
        i18n::text(self.cx.preferences.locale, text)
    }
    fn remember(&mut self, id: impl Into<String>, response: &egui::Response, glyph: Option<Icon>) {
        let id = id.into();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(&id));
        self.actions.push(Action::Record(Box::new(Recorded {
            id,
            response: response.clone(),
            glyph,
            tooltip: None,
        })));
    }
    fn icon(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        glyph: Icon,
        label: Text,
        enabled: bool,
    ) -> bool {
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                crate::kit::icon_button(ui, id, glyph, self.tr(label), false)
            })
            .inner;
        self.remember(id, &response, Some(glyph));
        response.clicked()
    }
    fn search(&mut self, ui: &mut egui::Ui, width: f32) {
        let hint = self.tr(match self.cx.preferences.assets_tab {
            AssetsTab::Transitions => Text::TransitionSearch,
            AssetsTab::Effects => Text::EffectSearch,
            _ => Text::AssetsSearch,
        });
        let draft = self
            .state
            .searches
            .entry(self.cx.preferences.assets_tab)
            .or_default();
        let response = ui.add_sized(
            [width.max(24.0), theme::size::CONTROL],
            egui::TextEdit::singleline(draft)
                .char_limit(256)
                .hint_text(hint)
                .font(egui::FontId::proportional(theme::type_size::BODY)),
        );
        self.remember("assets.search", &response, None);
    }
    fn empty(&mut self, ui: &mut egui::Ui, glyph: Icon, label: Text) {
        let rect = ui.available_rect_before_wrap();
        let center = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.36);
        icons::tinted(glyph, 32.0, theme::secondary(self.cx.preferences.dark))
            .paint_at(ui, Rect::from_center_size(center, Vec2::splat(32.0)));
        ui.painter().text(
            center + Vec2::new(0.0, 32.0),
            Align2::CENTER_TOP,
            self.tr(label),
            FontId::proportional(theme::type_size::BODY),
            theme::secondary(self.cx.preferences.dark),
        );
    }
}

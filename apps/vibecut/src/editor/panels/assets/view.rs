//! Renders the assets surface from a read-only projection.
use crate::i18n;
mod browser;
mod captions;
mod deletion;
mod effects;
mod feedback;
mod music;
mod sounds;
mod stickers;
mod transition;

pub(in crate::editor) use deletion::show as delete_dialog;
pub(in crate::editor) use feedback::{drop_overlay, import_overlay, toast};

use super::{
    Action, Align2, AssetRow, AssetState, AssetsTab, AudioSection, Color32, EffectRow, FontId,
    Icon, Id, MediaKind, MediaSortKey, MediaSortOrder, MediaView, Model, Recorded, Rect, Row,
    State, Text, Vec2, asset_state_color, egui, icons, state, theme,
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
    // The captions pages are always reachable, so their sub-navigation never collapses.
    let captions_tab = cx.preferences.assets_tab == AssetsTab::Captions;
    let nav = Rect::from_min_size(
        body.min,
        Vec2::new(
            if rect.width() < 420.0 && !captions_tab {
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
        AssetsTab::Stickers => Text::Stickers,
        AssetsTab::Effects => Text::Effects,
        tab => tab.label(),
    };
    if nav.width() > 0.0 {
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(nav.shrink2(Vec2::new(8.0, 15.0))),
            |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                if captions_tab {
                    view.captions_nav(ui);
                    return;
                }
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
                if cx.preferences.assets_tab == AssetsTab::Media {
                    // Import heads a group (caret, neutral); the shown view, Media, is the one
                    // active row.
                    let response = view.nav_row(ui, label, false, true);
                    view.remember("assets.subnav", &response, None);
                    let label = view.tr(Text::Media);
                    let response = view.nav_row(ui, label, true, false);
                    view.remember("assets.subnav.media", &response, None);
                    return;
                }
                let is_text = cx.preferences.assets_tab == AssetsTab::Text;
                let response = view.nav_row(ui, label, !is_text, false);
                if is_text {
                    view.remember("text.insert", &response, None);
                    if response.clicked() {
                        view.actions.push(Action::Command("text.insert".into()));
                    }
                } else {
                    view.remember("assets.subnav", &response, None);
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
            AssetsTab::Captions => view.actions.push(Action::Captions(content)),
            AssetsTab::Effects => view.effects(ui),
            AssetsTab::Transitions => view.transitions(ui),
            AssetsTab::Stickers => view.stickers(ui),
            AssetsTab::Adjustment => view.adjustment(ui),
            AssetsTab::Text => {
                if nav.width() == 0.0
                    && view.icon(ui, "text.insert", Icon::TextT, Text::AssetsAddText, true)
                {
                    view.actions.push(Action::Command("text.insert".into()));
                }
                view.empty(
                    ui,
                    Icon::TextT,
                    Text::AssetsNoStyles,
                    crate::kit::Tone::Quiet,
                );
            }
            tab => {
                let _ = tab;
                view.empty(
                    ui,
                    Icon::FolderOpen,
                    Text::AssetsUnavailable,
                    crate::kit::Tone::Quiet,
                );
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
    fn adjustment(&mut self, ui: &mut egui::Ui) {
        let presets = self.cx.preferences.adjustment_presets.clone();
        if presets.is_empty() {
            self.empty(
                ui,
                Icon::SlidersHorizontal,
                Text::AdjustmentPresetsEmpty,
                crate::kit::Tone::Quiet,
            );
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
    /// One 104x24 sub-navigation row on the hover fill: the selected row takes the accent
    /// label, and a group row carries a caret at the right.
    fn nav_row(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        selected: bool,
        group: bool,
    ) -> egui::Response {
        let dark = self.cx.preferences.dark;
        let response = ui.add_sized(
            [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
            egui::Button::new(
                egui::RichText::new(label)
                    .font(theme::label_font())
                    .size(theme::type_size::SMALL_LABEL)
                    .color(if selected {
                        theme::accent(dark)
                    } else {
                        theme::text(dark)
                    }),
            )
            .fill(theme::hover(dark))
            .stroke(egui::Stroke::NONE)
            .right_text(""),
        );
        if group {
            let caret = Rect::from_center_size(
                egui::pos2(response.rect.right() - 12.0, response.rect.center().y),
                Vec2::splat(12.0),
            );
            icons::tinted(Icon::CaretDown, 12.0, theme::text(dark)).paint_at(ui, caret);
        }
        response
    }
    fn search(&mut self, ui: &mut egui::Ui, width: f32) {
        let hint = self.tr(match self.cx.preferences.assets_tab {
            AssetsTab::Transitions => Text::TransitionSearch,
            AssetsTab::Effects => Text::EffectSearch,
            AssetsTab::Stickers => Text::StickerSearch,
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
    fn empty(&mut self, ui: &mut egui::Ui, glyph: Icon, label: Text, tone: crate::kit::Tone) {
        let rect = ui.available_rect_before_wrap();
        crate::kit::panel_state(ui, "assets.state", rect, glyph, self.tr(label), tone);
    }
}

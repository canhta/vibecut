//! Presents the transition catalog in the measured asset grid.
use super::super::transition::{self, Preview};
use super::{
    Action, Align2, AssetsTab, FontId, Icon, Id, Rect, Text, Vec2, View, egui, icons, theme,
};

impl View<'_> {
    pub(super) fn transition_nav(&mut self, ui: &mut egui::Ui) {
        const ORDER: &[&str] = &[
            "Basic",
            "Overlay",
            "Light",
            "Blur",
            "Camera",
            "3D",
            "Slides",
            "Wipes",
            "Glitch & Distort",
        ];
        let mut shelves: Vec<&'static str> = Vec::new();
        for definition in project::TRANSITION_DEFINITIONS {
            if !shelves.contains(&definition.shelf) {
                shelves.push(definition.shelf);
            }
        }
        shelves.sort_by_key(|shelf| {
            ORDER
                .iter()
                .position(|known| known == shelf)
                .unwrap_or(usize::MAX)
        });
        let rows = std::iter::once((None, Text::Transitions)).chain(
            shelves
                .into_iter()
                .map(|shelf| (Some(shelf), transition::shelf_label(shelf))),
        );
        for (family, label) in rows {
            let selected = self.state.transition_family == family;
            let response = ui
                .add_sized(
                    [theme::size::SUBNAV_BUTTON, theme::size::CONTROL],
                    egui::Button::new(
                        egui::RichText::new(self.tr(label))
                            .size(theme::type_size::CHROME)
                            .color(if selected {
                                theme::accent(self.cx.preferences.dark)
                            } else {
                                theme::text(self.cx.preferences.dark)
                            }),
                    )
                    .fill(if selected {
                        theme::hover(self.cx.preferences.dark)
                    } else {
                        theme::panel(self.cx.preferences.dark)
                    })
                    .stroke(egui::Stroke::NONE)
                    .truncate()
                    .right_text(""),
                )
                .on_hover_text(self.tr(label));
            self.remember(
                format!("transitions.category.{}", family.unwrap_or("all")),
                &response,
                None,
            );
            if response.clicked() {
                self.state.transition_family = family;
            }
        }
    }
    pub(super) fn transitions(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        self.search(ui, area.width());
        let query = self
            .state
            .searches
            .get(&AssetsTab::Transitions)
            .map(|query| query.to_lowercase())
            .unwrap_or_default();
        let definitions: Vec<_> = project::TRANSITION_DEFINITIONS
            .iter()
            .filter(|definition| {
                self.state
                    .transition_family
                    .is_none_or(|shelf| shelf == definition.shelf)
            })
            .filter(|definition| {
                self.tr(crate::i18n::transition_label(definition.kind))
                    .to_lowercase()
                    .contains(&query)
            })
            .collect();
        if let Some(shelf) = self.state.transition_family {
            ui.painter().text(
                area.min + Vec2::new(0.0, 36.0),
                Align2::LEFT_TOP,
                self.tr(transition::shelf_label(shelf)),
                FontId::proportional(theme::type_size::BODY),
                theme::text(self.cx.preferences.dark),
            );
        }
        let width = theme::size::EFFECT_CARD;
        let columns = ((area.width() + theme::space::GRID) / (width + theme::space::GRID))
            .floor()
            .max(1.0) as usize;
        let cards = Rect::from_min_max(area.min + Vec2::new(0.0, 60.0), area.max);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("transitions.results")
                .max_rect(cards),
            |ui| {
                if definitions.is_empty() {
                    self.empty(
                        ui,
                        Icon::ArrowsLeftRight,
                        Text::AssetsNoMatches,
                        crate::kit::Tone::Quiet,
                    );
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("transitions.tiles")
                    .show(ui, |ui| {
                        let (content, _) = ui.allocate_exact_size(
                            Vec2::new(
                                ui.available_width(),
                                definitions.len().div_ceil(columns) as f32
                                    * theme::size::EFFECT_PITCH,
                            ),
                            egui::Sense::hover(),
                        );
                        for (index, definition) in definitions.iter().enumerate() {
                            let rect = Rect::from_min_size(
                                content.min
                                    + Vec2::new(
                                        (index % columns) as f32 * (width + theme::space::GRID),
                                        (index / columns) as f32 * theme::size::EFFECT_PITCH,
                                    ),
                                Vec2::new(width, width + 22.0),
                            );
                            self.transition_tile(ui, definition, rect);
                        }
                    });
            },
        );
    }
    fn transition_tile(
        &mut self,
        ui: &mut egui::Ui,
        definition: &project::TransitionDefinition,
        rect: Rect,
    ) {
        let kind = definition.kind;
        let name = self.tr(crate::i18n::transition_label(definition.kind));
        let reason = self.cx.transition_refusal.map(|reason| self.tr(reason));
        let enabled = self.cx.transition_drag_enabled;
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.interact(
                    rect,
                    Id::unique(("transitions.tile", kind)),
                    egui::Sense::click_and_drag(),
                )
            })
            .inner;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
        let response = response
            .on_hover_text(reason.unwrap_or(name))
            .on_disabled_hover_text(reason.unwrap_or(name));
        self.remember(format!("transitions.tile.{kind}"), &response, None);
        if let Some(Action::Record(record)) = self.actions.last_mut() {
            record.tooltip = Some(reason.unwrap_or(name).into());
        }
        if response.drag_started() {
            self.actions.push(Action::TransitionDrag(kind));
        }
        if response.clicked() && reason.is_none() {
            self.actions.push(Action::AddTransition(kind));
        }

        let preview = Rect::from_min_size(rect.min, Vec2::splat(theme::size::EFFECT_CARD));
        ui.painter().rect_filled(
            preview,
            theme::radius::UPLOAD,
            theme::field(self.cx.preferences.dark),
        );
        if self
            .state
            .transition_previews
            .get(kind)
            .is_some_and(|preview| preview.dark != self.cx.preferences.dark)
        {
            self.state.transition_previews.remove(kind);
        }
        if !self.state.transition_previews.contains_key(kind) {
            match Preview::new(kind, self.cx.preferences.dark) {
                Ok(preview) => {
                    self.state.transition_previews.insert(kind, preview);
                }
                Err(error) => {
                    tracing::warn!(category = "transition_preview", %error, "preview unavailable");
                }
            }
        }
        let active = ui.is_rect_visible(rect)
            && ui.input(|input| input.focused)
            && (response.contains_pointer() || response.has_focus());
        let mut failed = true;
        let mut progress = 0.5;
        if let Some(entry) = self.state.transition_previews.get_mut(kind) {
            entry.update(ui.ctx(), kind, active);
            progress = entry.progress;
            failed = entry.failed;
            if let Some(texture) = entry.texture() {
                egui::Image::new((texture.id(), preview.size()))
                    .corner_radius(theme::radius::UPLOAD)
                    .paint_at(ui, preview);
            }
            if entry.failed {
                icons::tinted(
                    Icon::FilmStrip,
                    24.0,
                    theme::secondary(self.cx.preferences.dark),
                )
                .paint_at(
                    ui,
                    Rect::from_center_size(preview.center(), Vec2::splat(24.0)),
                );
            }
        }
        let status = if failed { "failed" } else { "ready" };
        let status_response = ui.interact(
            preview.shrink(24.0),
            Id::unique(("transitions.preview", kind)),
            egui::Sense::hover(),
        );
        status_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, name));
        ui.ctx().accesskit_node_builder(status_response.id, |node| {
            node.set_value(format!("{:.0}%", progress * 100.));
        });
        self.remember(
            format!("transitions.preview.{kind}.{status}"),
            &status_response,
            None,
        );
        if failed {
            let retry_rect = Rect::from_min_size(
                preview.left_bottom() - Vec2::new(0.0, 24.0),
                Vec2::splat(24.0),
            );
            let retry = ui
                .interact(
                    retry_rect,
                    Id::unique(("transitions.preview.retry", kind)),
                    egui::Sense::click(),
                )
                .on_hover_text(self.tr(Text::TransitionPreviewRetry));
            retry.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    self.tr(Text::TransitionPreviewRetry),
                )
            });
            self.remember(
                format!("transitions.preview.retry.{kind}"),
                &retry,
                Some(Icon::ArrowClockwise),
            );
            icons::tinted(
                Icon::ArrowClockwise,
                16.0,
                theme::text(self.cx.preferences.dark),
            )
            .paint_at(ui, retry_rect.shrink(4.0));
            if retry.clicked() {
                self.state.transition_previews.remove(kind);
            }
        }
        if response.hovered() || response.has_focus() {
            ui.painter().rect_stroke(
                preview,
                theme::radius::UPLOAD,
                egui::Stroke::new(1.0, theme::accent(self.cx.preferences.dark)),
                egui::StrokeKind::Inside,
            );
        }
        let plus = Rect::from_min_size(
            preview.right_bottom() - Vec2::splat(24.0),
            Vec2::splat(24.0),
        );
        let response = ui
            .add_enabled_ui(reason.is_none(), |ui| {
                ui.interact(
                    plus,
                    Id::unique(("transitions.add", kind)),
                    egui::Sense::click(),
                )
            })
            .inner;
        let label = reason.unwrap_or(self.tr(Text::TransitionAdd));
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, reason.is_none(), label));
        let response = response.on_hover_text(label).on_disabled_hover_text(label);
        self.remember(
            format!("transitions.add.{kind}"),
            &response,
            Some(Icon::Plus),
        );
        if let Some(Action::Record(record)) = self.actions.last_mut() {
            record.tooltip = Some(label.into());
        }
        ui.painter()
            .circle_filled(plus.center(), 8.0, theme::scrim());
        icons::tinted(
            Icon::Plus,
            12.0,
            if reason.is_none() {
                theme::on_scrim()
            } else {
                theme::secondary(self.cx.preferences.dark)
            },
        )
        .paint_at(ui, plus.shrink(6.0));
        if response.clicked() {
            self.actions.push(Action::AddTransition(kind));
        }
        ui.painter().with_clip_rect(rect).text(
            preview.left_bottom() + Vec2::new(0.0, theme::space::XS),
            Align2::LEFT_TOP,
            name,
            FontId::proportional(theme::type_size::BODY),
            theme::text(self.cx.preferences.dark),
        );
    }
}

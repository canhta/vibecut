//! Renders effect choices.

use crate::editor::assets::{
    Align2, BUTTON_ICON, Color32, Editor, FontId, Icon, Id, Rect, Text, Vec2,
    effect_preview_params, egui, icons, theme,
};

impl Editor {
    pub(in crate::editor) fn effects_view(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Effects))
                .size(16.0)
                .strong(),
        );
        ui.add_space(theme::space::XS);
        ui.label(
            egui::RichText::new(self.tr(Text::EffectDropHint))
                .small()
                .color(self.secondary()),
        );
        ui.add_space(theme::space::MD);
        let width = ((ui.available_width() - 8.0) / 2.0).clamp(112.0, 148.0);
        egui::ScrollArea::vertical()
            .id_salt("effects.tiles")
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for definition in project::effect_definitions() {
                        self.effect_tile(ui, definition, width);
                    }
                });
            });
    }
    pub(in crate::editor) fn effect_tile(
        &mut self,
        ui: &mut egui::Ui,
        definition: &project::EffectDefinition,
        width: f32,
    ) {
        let kind = definition.kind;
        let params = effect_preview_params(definition);
        if let (Some(previews), Some(params)) = (&mut self.effect_previews, params.as_ref()) {
            previews.ensure(kind, params);
        }
        let status = self
            .effect_previews
            .as_ref()
            .and_then(|previews| previews.status(kind));
        if !matches!(
            status,
            Some(crate::effect_previews::EffectPreviewStatus::Ready)
        ) {
            self.effect_textures.remove(kind);
        } else if !self.effect_textures.contains_key(kind)
            && let Some(image) = self
                .effect_previews
                .as_ref()
                .and_then(|previews| previews.image(kind))
        {
            let texture = crate::effect_previews::preview_texture(ui.ctx(), kind, image);
            self.effect_textures.insert(kind.to_owned(), texture);
        }
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, 118.0), egui::Sense::click_and_drag());
        self.remember(format!("effects.tile.{kind}"), &response);
        let state = match status {
            Some(crate::effect_previews::EffectPreviewStatus::Pending) => "pending",
            Some(crate::effect_previews::EffectPreviewStatus::Ready) => "ready",
            Some(crate::effect_previews::EffectPreviewStatus::Failed) => "failed",
            None => "failed",
        };
        self.remember(format!("effects.tile.{kind}.{state}"), &response);
        if response.drag_started() {
            self.effect_drag = Some(kind.to_owned());
        }
        ui.painter()
            .rect_filled(rect, theme::radius::CONTROL, self.control_fill());
        let preview = Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, rect.top() + 6.0),
            egui::pos2(rect.right() - 6.0, rect.bottom() - 26.0),
        );
        match status {
            Some(crate::effect_previews::EffectPreviewStatus::Ready) => {
                if let Some(texture) = self.effect_textures.get(kind) {
                    ui.painter().image(
                        texture.id(),
                        preview,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            Some(crate::effect_previews::EffectPreviewStatus::Pending) => {
                ui.painter().text(
                    preview.center(),
                    Align2::CENTER_CENTER,
                    self.tr(Text::EffectPreviewPending),
                    FontId::proportional(theme::type_size::BODY),
                    self.secondary(),
                );
            }
            Some(crate::effect_previews::EffectPreviewStatus::Failed) | None => {
                ui.painter().text(
                    preview.center(),
                    Align2::CENTER_CENTER,
                    self.tr(Text::EffectPreviewFailed),
                    FontId::proportional(theme::type_size::BODY),
                    self.secondary(),
                );
                let retry = Rect::from_center_size(
                    egui::pos2(preview.right() - 16.0, preview.bottom() - 16.0),
                    Vec2::splat(theme::size::CONTROL),
                );
                let response = ui.interact(
                    retry,
                    Id::unique(("effects.preview.retry", kind)),
                    egui::Sense::click(),
                );
                let label = self.tr(Text::EffectPreviewRetry);
                icons::tinted(Icon::ArrowClockwise, BUTTON_ICON, self.secondary())
                    .paint_at(ui, retry);
                let response = response.on_hover_text(label);
                self.remember_with_icon(
                    format!("effects.preview.retry.{kind}"),
                    Some(Icon::ArrowClockwise.name()),
                    &response,
                );
                if response.clicked() {
                    if let (Some(previews), Some(params)) =
                        (&mut self.effect_previews, params.as_ref())
                    {
                        previews.retry(kind, params);
                    }
                    self.effect_textures.remove(kind);
                }
            }
        }
        let plus = Rect::from_center_size(
            egui::pos2(rect.right() - 15.0, rect.top() + 15.0),
            Vec2::splat(theme::size::CONTROL),
        );
        let response = ui.interact(
            plus,
            Id::unique(("effects.add", kind)),
            egui::Sense::click(),
        );
        let label = self.tr(Text::EffectAdd);
        ui.painter().circle_filled(
            plus.center(),
            9.0,
            self.accent()
                .gamma_multiply(if response.hovered() { 1.0 } else { 0.8 }),
        );
        icons::tinted(Icon::Plus, 12.0, theme::on_accent(self.preferences.dark))
            .paint_at(ui, plus.shrink(6.0));
        let response = response.on_hover_text(label);
        self.remember_with_icon(
            format!("effects.add.{kind}"),
            Some(Icon::Plus.name()),
            &response,
        );
        if response.clicked() {
            self.add_effect_from_selection(kind);
        }
        ui.painter().text(
            egui::pos2(rect.left() + 8.0, rect.bottom() - 14.0),
            Align2::LEFT_CENTER,
            self.effect_name(definition),
            FontId::proportional(theme::type_size::BODY),
            self.text_color(),
        );
    }
    pub(in crate::editor) fn effect_name(
        &self,
        definition: &project::EffectDefinition,
    ) -> &'static str {
        match definition.kind {
            "blur" => self.tr(Text::EffectBlur),
            _ => definition.name,
        }
    }
}

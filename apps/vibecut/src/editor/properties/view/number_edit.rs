//! Renders numeric property editing.
use crate::editor::properties::{
    BUTTON_ICON, Editor, Error, Icon, Id, ItemId, Rect, ScrubSpec, ScrubTarget, Text, Vec2, egui,
    icons, theme,
};

impl Editor {
    /// A numeric field whose left icon scrubs, whose text commits on Enter or
    /// blur, and whose gesture is one engine undo step from preview to release.
    pub(in crate::editor) fn scrub_field(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        spec: ScrubSpec,
    ) {
        let Some(current) = self.item_value(item, spec.field) else {
            return;
        };
        ui.add_space(theme::space::LG);
        let caption = ui.label(self.tr(spec.label));
        ui.horizontal(|ui| {
            let handle_id = format!("inspector.{}.scrub", spec.field);
            let (rect, handle) = ui.allocate_exact_size(
                Vec2::new(theme::size::CONTROL, theme::size::FIELD),
                egui::Sense::click_and_drag(),
            );
            if handle.hovered() || handle.dragged() {
                ui.painter()
                    .rect_filled(rect, theme::radius::CONTROL, self.control_fill());
            }
            icons::tinted(Icon::ArrowsHorizontal, BUTTON_ICON, self.secondary()).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
            );
            let handle = handle.on_hover_text(self.tr(Text::DragToAdjust));
            self.remember_with_icon(
                handle_id.clone(),
                Some(Icon::ArrowsHorizontal.name()),
                &handle,
            );
            if handle.drag_started() {
                self.start_scrub(item, spec, current);
            }
            if handle.dragged()
                && let Some(mut scrub) = self.scrub.take()
            {
                if scrub.target == ScrubTarget::Item(spec.field) {
                    let steps = f64::from(handle.total_drag_delta().map_or(0.0, |delta| delta.x))
                        / f64::from(spec.pixels_per_step);
                    let value = spec.quantize(scrub.start + steps * spec.step);
                    if value != scrub.last {
                        match self.set_item_field(item, spec.field, &spec.format(value)) {
                            Ok(()) => scrub.last = value,
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                self.scrub = Some(scrub);
            }
            if handle.drag_stopped() {
                self.finish_scrub(true);
            }
            let transform = project::TransformField::ALL
                .into_iter()
                .find(|f| f.path() == spec.field);
            let width = (ui.available_width()
                - if spec.suffix.is_empty() { 0.0 } else { 30.0 }
                - if transform.is_some() { 32.0 } else { 0.0 })
            .max(40.0);
            let id = format!("inspector.{}", spec.field);
            let mut next = self
                .property_drafts
                .get(spec.field)
                .cloned()
                .unwrap_or_else(|| spec.format(current));
            let response = ui
                .add_sized(
                    [width, 26.0],
                    egui::TextEdit::singleline(&mut next)
                        .char_limit(32)
                        .background_color(self.control_fill())
                        .id(Id::unique(&id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
                .labelled_by(caption.id);
            self.remember(id.clone(), &response);
            if response.has_focus() {
                self.property_focus = Some(spec.field.to_owned());
            }
            if response.changed() {
                self.property_drafts.insert(spec.field.to_owned(), next);
            }
            if !spec.suffix.is_empty() {
                ui.label(
                    egui::RichText::new(spec.suffix)
                        .small()
                        .color(self.secondary()),
                );
            }
            if let Some(field) = transform {
                let id = format!("inspector.{}.reset", field.path());
                let reset = ui
                    .add(egui::Button::image(icons::tinted(
                        Icon::ArrowCounterClockwise,
                        BUTTON_ICON,
                        self.secondary(),
                    )))
                    .on_hover_text(self.tr(Text::ResetValue));
                self.remember_with_icon(id, Some(Icon::ArrowCounterClockwise.name()), &reset);
                if reset.clicked() {
                    self.property_drafts.remove(spec.field);
                    self.property_focus = None;
                    response.surrender_focus();
                    if let Err(error) =
                        self.set_item_field(item, field.path(), &field.default_value().to_string())
                    {
                        self.error = Some(error.to_string());
                    }
                    return;
                }
            }
            let editing =
                response.has_focus() || self.property_focus.as_deref() == Some(spec.field);
            if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.property_drafts.remove(spec.field);
                self.property_focus = None;
                response.surrender_focus();
                return;
            }
            let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || response.lost_focus() {
                self.property_focus = None;
                if let Some(draft) = self.property_drafts.remove(spec.field) {
                    let Ok(parsed) = draft.trim().parse::<f64>() else {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    };
                    if !parsed.is_finite() {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    }
                    let value = spec.quantize(parsed);
                    if let Err(error) = self.set_item_field(item, spec.field, &spec.format(value)) {
                        self.error = Some(error.to_string());
                    }
                }
            }
        });
    }
}

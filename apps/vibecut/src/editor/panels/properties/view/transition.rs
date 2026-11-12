//! Renders the selected transition's duration, alignment and removal.
use super::{INSET, Panel, body_ui};
use crate::editor::properties::{
    Action, Rect, TRANSITION_DURATION_SCRUB, Text, egui, icons, theme,
};
use crate::kit;
use project::{Transition, TransitionAlignment as Align};
use time::{Rounding, Tick};

const FIELD_WIDTH: f32 = 68.0;
const DRAFT: &str = "transition_duration";

impl Panel<'_, '_> {
    pub(super) fn transition_panel(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        transition: project::TransitionId,
    ) {
        if self.state.focus.as_deref() != Some(DRAFT) {
            self.state.focus = None;
        }
        let found = self
            .cx
            .session
            .project()
            .data()
            .scene()
            .timeline
            .transitions
            .iter()
            .find(|tr| tr.id == transition)
            .cloned();
        let ids = ["inspector.tab.transition".to_owned()];
        let labels = [self.tr(Text::TransitionInspector)];
        let tabs = kit::text_tabs(ui, &ids, &labels, 0);
        for (id, response) in ids.iter().zip(&tabs) {
            self.remember(id.clone(), response);
        }
        let Some(tr) = found else {
            return;
        };
        let mut body = body_ui(ui, area);
        egui::ScrollArea::vertical()
            .id_salt("inspector.transition")
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                ui.add_space(INSET / 2.0);
                let name = self.tr(crate::editor::assets::transition_label(&tr.kind));
                self.detail_row(ui, name);
                self.transition_duration(ui, &tr);
                self.transition_alignment(ui, &tr);
                ui.add_space(INSET);
                self.transition_remove(ui, &tr);
            });
    }

    /// Draft field (commit on Enter or blur, Escape cancels) with its stepper.
    fn transition_duration(&mut self, ui: &mut egui::Ui, tr: &Transition) {
        let spec = TRANSITION_DURATION_SCRUB;
        let current = tr.duration.seconds();
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(spec.label));
        value_ui.spacing_mut().item_spacing.x = theme::space::XXS;
        let mut next = self
            .state
            .drafts
            .get(DRAFT)
            .cloned()
            .unwrap_or_else(|| format!("{}{}", spec.format(current), spec.suffix));
        let label = self.tr(spec.label);
        let response = kit::value_field(
            &mut value_ui,
            "inspector.transition.duration",
            &mut next,
            FIELD_WIDTH,
            None,
            label,
        );
        self.remember("inspector.transition.duration", &response);
        if response.has_focus() {
            self.state.focus = Some(DRAFT.to_owned());
        }
        if response.changed() {
            self.state.drafts.insert(DRAFT.to_owned(), next);
        }
        value_ui.add_space(4.0);
        let (up, down) = kit::stepper(&mut value_ui, "inspector.transition.duration", label);
        self.remember("inspector.transition.duration.increase", &up);
        self.remember("inspector.transition.duration.decrease", &down);
        for (clicked, sign) in [(up.clicked(), 1.0), (down.clicked(), -1.0)] {
            if clicked {
                self.state.drafts.remove(DRAFT);
                self.set_duration(tr, spec.quantize(current + sign * spec.step));
            }
        }
        let editing = response.has_focus() || self.state.focus.as_deref() == Some(DRAFT);
        if editing && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.state.drafts.remove(DRAFT);
            self.state.focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.state.focus = None;
            if let Some(draft) = self.state.drafts.remove(DRAFT) {
                match parse(&draft) {
                    Some(seconds) => self.set_duration(tr, spec.quantize(seconds)),
                    None => self.fail(crate::editor::Error::Invalid("transition duration")),
                }
            }
        }
    }

    fn set_duration(&mut self, tr: &Transition, seconds: f64) {
        let Ok(duration) = Tick::from_seconds_f64(seconds, Rounding::Nearest) else {
            return;
        };
        if duration != tr.duration {
            self.actions.push(Action::SetTransitionDuration {
                transition: tr.id,
                duration,
            });
        }
    }

    fn transition_alignment(&mut self, ui: &mut egui::Ui, tr: &Transition) {
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::TransitionAlignment));
        let ids = [
            "inspector.transition.align.start".to_owned(),
            "inspector.transition.align.center".to_owned(),
            "inspector.transition.align.end".to_owned(),
        ];
        let labels = [
            self.tr(Text::TransitionStart),
            self.tr(Text::TransitionCenter),
            self.tr(Text::TransitionEnd),
        ];
        // A span moved by hand matches none of the three.
        let selected = match tr.alignment {
            Align::Start => 0,
            Align::Center => 1,
            Align::End => 2,
            Align::Custom => labels.len(),
        };
        let cells = kit::segmented(&mut value_ui, &ids, &labels, selected);
        for ((id, response), alignment) in
            ids.iter()
                .zip(&cells)
                .zip([Align::Start, Align::Center, Align::End])
        {
            self.remember(id.clone(), response);
            if response.clicked() && tr.alignment != alignment {
                self.actions.push(Action::SetTransitionAlignment {
                    transition: tr.id,
                    alignment,
                });
            }
        }
    }

    fn transition_remove(&mut self, ui: &mut egui::Ui, tr: &Transition) {
        let width = ui.available_width();
        let response = ui
            .push_id("inspector.transition.remove", |ui| {
                ui.add_sized(
                    [width, theme::size::CONTROL],
                    egui::Button::image_and_text(
                        icons::tinted(kit::Icon::Trash, 16.0, theme::text(self.cx.dark)),
                        self.tr(Text::TransitionRemove),
                    ),
                )
            })
            .inner;
        self.remember("inspector.transition.remove", &response);
        if response.clicked() {
            self.actions.push(Action::RemoveTransition(tr.id));
        }
    }
}

fn parse(text: &str) -> Option<f64> {
    let value = text
        .trim()
        .trim_end_matches('s')
        .trim()
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value)
}

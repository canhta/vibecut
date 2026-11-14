//! Draws the footage-correction rows: the Stabilize switch with its level, the Remove flicker
//! switch, and the analysis state row they share with Auto removal.
use super::Panel;
use crate::editor::properties::{
    Action, ItemId, Text, egui, read, theme, workflow::jobs::footage::FootageStatus,
};
use crate::kit;
use engine::footage::CorrectionKind;

impl Panel<'_, '_> {
    /// The Stabilize switch row, then, while it is on, the level row and the analysis state.
    /// Turning the switch on starts the job; a warm cache finishes at once.
    pub(super) fn stabilize_rows(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Video(params)) =
            read::item(self.cx.session.project(), item).map(|item| &item.content)
        else {
            return;
        };
        let level = params.stabilize;
        let mut on = level.is_some();
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::Stabilize));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(
            &mut value_ui,
            "inspector.stabilize",
            &mut on,
            self.tr(Text::Stabilize),
        );
        self.remember("inspector.stabilize", &response);
        if response.clicked() {
            if on {
                self.set_item_field(item, "stabilize", project::StabilizeLevel::default().id());
                self.actions.push(Action::AnalyzeFootage {
                    item,
                    kind: CorrectionKind::Stabilize,
                });
            } else {
                self.set_item_field(item, "stabilize", "off");
            }
        }
        let Some(current) = level else {
            return;
        };
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::StabilizeLevel));
        let ids: Vec<String> = project::StabilizeLevel::ALL
            .iter()
            .map(|level| format!("inspector.stabilize.level.{}", level.id()))
            .collect();
        let labels: Vec<&str> = project::StabilizeLevel::ALL
            .iter()
            .map(|level| self.tr(stabilize_label(*level)))
            .collect();
        let selected = project::StabilizeLevel::ALL
            .iter()
            .position(|level| *level == current)
            .unwrap_or(0);
        let responses = kit::segmented(&mut value_ui, &ids, &labels, selected);
        for ((level, id), response) in project::StabilizeLevel::ALL
            .iter()
            .zip(&ids)
            .zip(&responses)
        {
            self.remember(id.clone(), response);
            if response.clicked() && *level != current {
                self.set_item_field(item, "stabilize", level.id());
            }
        }
        self.correction_status(ui, item, CorrectionKind::Stabilize, "inspector.stabilize");
    }

    /// The Remove flicker switch row and, while it is on, the analysis state. Turning the switch
    /// on starts the brightness measurement; a warm cache finishes at once. The reference shows
    /// no level, so the row is the whole control.
    pub(super) fn flicker_rows(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Video(params)) =
            read::item(self.cx.session.project(), item).map(|item| &item.content)
        else {
            return;
        };
        let mut on = params.remove_flicker;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::RemoveFlicker));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(
            &mut value_ui,
            "inspector.remove_flicker",
            &mut on,
            self.tr(Text::RemoveFlicker),
        );
        self.remember("inspector.remove_flicker", &response);
        if response.clicked() {
            self.set_item_field(item, "remove_flicker", if on { "true" } else { "false" });
            if on {
                self.actions.push(Action::AnalyzeFootage {
                    item,
                    kind: CorrectionKind::Flicker,
                });
            }
        }
        if on {
            self.correction_status(
                ui,
                item,
                CorrectionKind::Flicker,
                "inspector.remove_flicker",
            );
        }
    }

    /// The analysis state of one switched-on correction, under `prefix`: nothing when the
    /// correction is stored; `Needs analysis` with an `Analyze` button when it is not; `Person data
    /// missing` with a `Download` link to the model manager when its weights are missing; progress
    /// with a cancel while the job runs.
    pub(super) fn correction_status(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        kind: CorrectionKind,
        prefix: &str,
    ) {
        match self.state.footage_status(item, kind) {
            FootageStatus::Ready => {}
            FootageStatus::NeedsAnalysis => {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::FootageNeedsAnalysis));
                let response = value_ui.button(self.tr(Text::FootageAnalyze));
                self.remember(format!("{prefix}.analyze"), &response);
                if response.clicked() {
                    self.actions.push(Action::AnalyzeFootage { item, kind });
                }
            }
            FootageStatus::ModelMissing(id) => {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::FootageDataMissing));
                value_ui.spacing_mut().item_spacing.x = theme::space::MD;
                let models = value_ui.button(self.tr(Text::ModelDownload));
                self.remember(format!("{prefix}.models"), &models);
                if models.clicked() {
                    self.actions.push(Action::OpenModel(id.to_owned()));
                }
                let retry = value_ui.button(self.tr(Text::FootageAnalyze));
                self.remember(format!("{prefix}.analyze"), &retry);
                if retry.clicked() {
                    self.actions.push(Action::AnalyzeFootage { item, kind });
                }
            }
            FootageStatus::Analyzing(progress) => {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::FootageAnalyzing));
                let cancel = value_ui.button(self.tr(Text::Cancel));
                self.remember(format!("{prefix}.cancel"), &cancel);
                if cancel.clicked() {
                    self.actions.push(Action::CancelFootage { item, kind });
                }
                let bar = value_ui.add(
                    egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                        .desired_width(value_ui.available_width().max(0.0)),
                );
                self.remember(format!("{prefix}.progress"), &bar);
            }
        }
    }
}

fn stabilize_label(level: project::StabilizeLevel) -> Text {
    match level {
        project::StabilizeLevel::Flexible => Text::StabilizeFlexible,
        project::StabilizeLevel::Normal => Text::StabilizeNormal,
        project::StabilizeLevel::Stable => Text::StabilizeStable,
    }
}

//! Draws the Stabilize switch with its level and analysis state.
use super::Panel;
use crate::editor::properties::{
    Action, ItemId, Text, egui, read, theme, workflow::jobs::footage::FootageStatus,
};
use crate::kit;

impl Panel<'_, '_> {
    /// The Stabilize switch row, then, while it is on, the level row and the analysis state: a
    /// `Needs analysis` button when no correction is stored, progress with a cancel while the
    /// job runs. Turning the switch on starts the job; a warm cache finishes at once.
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
                self.actions.push(Action::AnalyzeFootage { item });
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
        match self.state.footage_status(item) {
            FootageStatus::Ready => {}
            FootageStatus::NeedsAnalysis => {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::StabilizeNeedsAnalysis));
                let response = value_ui.button(self.tr(Text::StabilizeAnalyze));
                self.remember("inspector.stabilize.analyze", &response);
                if response.clicked() {
                    self.actions.push(Action::AnalyzeFootage { item });
                }
            }
            FootageStatus::Analyzing(progress) => {
                let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
                self.row_label(ui, row, self.tr(Text::StabilizeAnalyzing));
                let cancel = value_ui.button(self.tr(Text::Cancel));
                self.remember("inspector.stabilize.cancel", &cancel);
                if cancel.clicked() {
                    self.actions.push(Action::CancelFootage { item });
                }
                let bar = value_ui.add(
                    egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                        .desired_width(value_ui.available_width().max(0.0)),
                );
                self.remember("inspector.stabilize.progress", &bar);
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

//! One engine-job stage line and progress bar for every job surface.
use crate::{
    egui,
    i18n::{self, Locale, Text},
};
use engine::jobs::{JobSnapshot, JobStage};

/// The export and shorts surfaces render the same engine stages and fraction.
pub fn job_progress(ui: &mut egui::Ui, snapshot: &JobSnapshot, locale: Locale) {
    let stage = match snapshot.stage {
        JobStage::Queued | JobStage::Starting => Text::ExportQueued,
        JobStage::Admit => Text::ExportAdmit,
        JobStage::Render => Text::ExportRender,
        JobStage::Mix => Text::ExportMix,
        JobStage::Encode => Text::ExportEncode,
        JobStage::Finalize => Text::ExportFinalize,
        JobStage::Publish => Text::ExportPublish,
        _ => Text::ExportProgress,
    };
    ui.label(i18n::text(locale, stage));
    ui.add(egui::ProgressBar::new(snapshot.progress).show_percentage());
}

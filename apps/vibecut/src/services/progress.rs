//! One status line with its percentage and progress bar for every job surface.
use crate::{
    egui,
    i18n::{self, Locale, Text},
};
use engine::jobs::{JobSnapshot, JobStage};

/// The export and shorts surfaces render the same engine stages and fraction.
pub fn job_progress(ui: &mut egui::Ui, id: &str, snapshot: &JobSnapshot, locale: Locale) {
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
    line(ui, id, locale, i18n::text(locale, stage), snapshot.progress);
}

/// A `status N%` line over the kit progress bar; the surface owns the status words.
pub fn line(ui: &mut egui::Ui, id: &str, locale: Locale, status: &str, fraction: f32) {
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).floor() as u64;
    ui.label(i18n::format(
        locale,
        Text::ProcessingPercent,
        &[
            ("status", status),
            ("percent", &i18n::number(locale, percent)),
        ],
    ));
    crate::kit::progress_bar(ui, id, fraction);
}

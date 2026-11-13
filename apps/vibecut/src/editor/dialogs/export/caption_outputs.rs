//! Renders the export dialog's caption-output buttons, one per saved output.
use super::dialog::{DialogAction, DialogUi};
use crate::i18n;
use crate::{
    egui,
    i18n::{Locale, Text},
    services::lifecycle::CaptionOutput,
};
use captions::Format;

const OUTPUTS: [(&str, Text, CaptionOutput); 4] = [
    (
        "export.captions.srt",
        Text::SubSrt,
        CaptionOutput::File(Format::Srt),
    ),
    (
        "export.captions.vtt",
        Text::SubVtt,
        CaptionOutput::File(Format::WebVtt),
    ),
    (
        "export.captions.txt",
        Text::SubTxt,
        CaptionOutput::File(Format::Txt),
    ),
    ("export.captions.burn", Text::SubMp4, CaptionOutput::Burned),
];

/// Caption outputs are offered only when the project holds a caption item; a click asks the
/// window for the native save dialog.
pub(super) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    project: &project::Project,
    result: &mut DialogUi,
) {
    let captioned = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .any(|item| matches!(item.content, project::ItemParams::Caption(_)));
    ui.label(egui::RichText::new(i18n::text(locale, Text::Caption)).strong());
    ui.horizontal(|ui| {
        for (id, label, output) in OUTPUTS {
            let response = ui.add_enabled(captioned, egui::Button::new(i18n::text(locale, label)));
            let response = response.on_disabled_hover_text(i18n::text(locale, Text::SubEmpty));
            if response.clicked() {
                result.action = Some(DialogAction::Caption(output));
            }
            result.widgets.push((id, response));
        }
    });
}

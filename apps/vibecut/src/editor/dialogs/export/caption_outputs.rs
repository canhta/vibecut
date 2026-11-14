//! Renders the export dialog's Captions section: a tickable caption file that joins the
//! export's group, and the buttons that save one caption output on its own.
use super::dialog::{DialogAction, DialogUi, ExportOptions};
use crate::i18n;
use crate::{
    egui,
    i18n::{Locale, Text},
    services::lifecycle::CaptionOutput,
};
use captions::Format;

/// The reference lists SRT and TXT; VTT and the burned-in MP4 follow as ours.
const OUTPUTS: [(&str, Text, CaptionOutput); 4] = [
    (
        "export.captions.srt",
        Text::SubSrt,
        CaptionOutput::File(Format::Srt),
    ),
    (
        "export.captions.txt",
        Text::SubTxt,
        CaptionOutput::File(Format::Txt),
    ),
    (
        "export.captions.vtt",
        Text::SubVtt,
        CaptionOutput::File(Format::WebVtt),
    ),
    ("export.captions.burn", Text::SubMp4, CaptionOutput::Burned),
];

/// The file formats the ticked section offers, with the widget id of each entry.
const FORMATS: [(&str, Text, Format); 3] = [
    ("export.captions.format.srt", Text::SubSrt, Format::Srt),
    ("export.captions.format.txt", Text::SubTxt, Format::Txt),
    ("export.captions.format.vtt", Text::SubVtt, Format::WebVtt),
];

/// Whether the project holds a caption item: the only case with anything to write.
pub(super) fn captioned(project: &project::Project) -> bool {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .any(|item| matches!(item.content, project::ItemParams::Caption(_)))
}

/// The Captions tick and its Format row, then the buttons. Ticked, the caption file is
/// written beside the saved media; a button asks the window for the native save dialog.
pub(super) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    project: &project::Project,
    options: &mut ExportOptions,
    result: &mut DialogUi,
) {
    let captioned = captioned(project);
    let ticked = options.also.captions.is_some();
    let mut value = ticked;
    let response = ui.add_enabled(
        captioned,
        egui::Checkbox::new(
            &mut value,
            egui::RichText::new(i18n::text(locale, Text::Caption)).strong(),
        ),
    );
    let response = response.on_disabled_hover_text(i18n::text(locale, Text::SubEmpty));
    if value != ticked {
        options.also.captions = value.then_some(Format::Srt);
    }
    result.widgets.push(("export.captions.include", response));
    ui.add_space(8.0);
    egui::Grid::new("export.captions")
        .num_columns(2)
        .min_col_width(115.0)
        .spacing([16.0, 16.0])
        .show(ui, |ui| {
            let row_label = ui.label(i18n::text(locale, Text::ExportFormat));
            let shown = options.also.captions.unwrap_or(Format::Srt);
            let label = FORMATS
                .iter()
                .find(|(_, _, format)| *format == shown)
                .map_or("", |(_, text, _)| i18n::text(locale, *text));
            let combo = ui
                .add_enabled_ui(ticked, |ui| {
                    egui::ComboBox::from_id_salt(egui::IdSalt::new("export.captions.format"))
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (id, text, format) in FORMATS {
                                let r =
                                    ui.selectable_label(shown == format, i18n::text(locale, text));
                                if r.clicked() {
                                    options.also.captions = Some(format);
                                }
                                result.widgets.push((id, r));
                            }
                        })
                        .response
                })
                .inner
                .on_disabled_hover_text(i18n::text(locale, Text::ExportTickCaptions));
            result
                .widgets
                .push(("export.captions.format", combo.labelled_by(row_label.id)));
            ui.end_row();
        });
    ui.add_space(8.0);
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

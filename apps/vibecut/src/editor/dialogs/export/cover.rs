//! The export dialog's cover: the project cover image with its Edit cover chip.
use super::dialog::{DialogAction, DialogUi};
use crate::i18n::{self, Locale, Text};
use crate::{egui, kit as theme};

/// The cover column's width, the 227 px of the export capture's cover column.
const WIDTH: f32 = 227.0;

/// What the dialog reads of the project cover: the stored image, and whether the session can
/// store another one.
#[derive(Clone, Copy)]
pub(crate) struct Cover<'a> {
    pub thumbnail: &'a project::Thumbnail,
    pub editable: bool,
}

/// The cover as last uploaded to the GPU, so the image is uploaded again only when it changes.
#[derive(Default)]
pub(super) struct Held(Option<(project::Thumbnail, egui::TextureHandle)>);

impl Held {
    fn texture(
        &mut self,
        ctx: &egui::Context,
        thumbnail: &project::Thumbnail,
    ) -> Option<&egui::TextureHandle> {
        let project::Thumbnail::Ready {
            width,
            height,
            rgba,
        } = thumbnail
        else {
            self.0 = None;
            return None;
        };
        if self.0.as_ref().is_none_or(|(held, _)| held != thumbnail) {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [*width as usize, *height as usize],
                rgba,
            );
            let texture = ctx.load_texture("export.cover", image, egui::TextureOptions::LINEAR);
            self.0 = Some((thumbnail.clone(), texture));
        }
        self.0.as_ref().map(|(_, texture)| texture)
    }
}

/// How the cover column is laid out and what it says.
pub(super) struct View {
    pub locale: Locale,
    pub dark: bool,
    /// Canvas height over width; the cover keeps the canvas ratio.
    pub aspect: f32,
    /// The height the form leaves for the column.
    pub height: f32,
    /// The timeline holds nothing, so there is no cover to wait for.
    pub empty: bool,
    /// An export is running: the chip is off while it runs.
    pub busy: bool,
}

/// Draw the project cover over the canvas ratio with the Edit cover chip at its top-left corner
/// [measured: frame 105340, 8 px in from the cover's top and left edges].
pub(super) fn show(
    ui: &mut egui::Ui,
    held: &mut Held,
    cover: Cover<'_>,
    view: View,
    result: &mut DialogUi,
) {
    let tr = |key| i18n::text(view.locale, key);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(WIDTH, (WIDTH * view.aspect).min(view.height)),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 4.0, theme::window(view.dark));
    if let Some(texture) = held.texture(ui.ctx(), cover.thumbnail) {
        let size = texture.size_vec2();
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let image = egui::Rect::from_center_size(rect.center(), size * scale);
        ui.painter().image(
            texture.id(),
            image,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        let waiting = if view.empty {
            Text::EmptyPreview
        } else if matches!(cover.thumbnail, project::Thumbnail::Unavailable) {
            Text::ExportCoverUnavailable
        } else {
            Text::Loading
        };
        let galley = ui.painter().layout(
            tr(waiting).to_owned(),
            egui::FontId::proportional(14.0),
            theme::secondary(view.dark),
            rect.width() - 24.0,
        );
        let position = rect.center() - galley.size() * 0.5;
        ui.painter()
            .galley(position, galley, theme::secondary(view.dark));
    }
    let chip = ui
        .add_enabled_ui(cover.editable && !view.busy, |ui| {
            theme::scrim_chip(
                ui,
                "export.cover.edit",
                theme::Icon::PencilSimple,
                tr(Text::ExportCoverEdit),
                rect.min + egui::Vec2::splat(theme::space::MD),
            )
        })
        .inner;
    let chip = if cover.editable {
        chip.on_hover_text(tr(Text::ExportCoverEditHint))
    } else {
        chip.on_disabled_hover_text(tr(Text::ExportCoverNeedsProject))
    };
    if chip.clicked() {
        result.action = Some(DialogAction::EditCover);
    }
    result.widgets.push(("export.cover.edit", chip));
}

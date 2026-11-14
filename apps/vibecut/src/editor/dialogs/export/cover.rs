//! The export dialog's cover: the project cover frame with its Edit cover chip.
//!
//! The stored cover is a 128 px thumbnail, too small for the 227 px column, so the dialog
//! shows it at once and renders the cover frame at the column's physical size off the UI
//! thread; the sharp frame replaces it when it arrives.
use super::dialog::{DialogAction, DialogUi};
use crate::i18n::{self, Locale, Text};
use crate::{egui, kit as theme};
use std::sync::{Arc, mpsc};

/// The cover column's width, the 227 px of the export capture's cover column.
const WIDTH: f32 = 227.0;

/// What the dialog reads of the project cover.
#[derive(Clone, Copy)]
pub(crate) struct Cover<'a> {
    /// The stored 128 px image, shown until the sized frame arrives.
    pub thumbnail: &'a project::Thumbnail,
    /// The output tick the cover shows.
    pub tick: time::Tick,
    /// The project snapshot the cover is drawn from.
    pub project: &'a Arc<project::Project>,
    /// The stored footage corrections the preview draws, read only when a render starts.
    pub corrections: &'a dyn Fn() -> render::Corrections,
    /// Whether the session can store another cover.
    pub editable: bool,
}

/// One sized render: the project snapshot, the tick and the pixel size.
#[derive(Clone)]
struct Key {
    project: Arc<project::Project>,
    tick: time::Tick,
    size: [u32; 2],
}

impl Key {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.project, &other.project)
            && self.tick == other.tick
            && self.size == other.size
    }
}

/// The cover's textures, so an image is uploaded again only when it changes, and the sized
/// render, which is kept until the project, tick or size changes (reopening the dialog on the
/// same project does not render again).
#[derive(Default)]
pub(super) struct Held {
    thumbnail: Option<(project::Thumbnail, egui::TextureHandle)>,
    sized: Option<(Key, egui::TextureHandle)>,
    pending: Option<(Key, mpsc::Receiver<Option<engine::cover::Image>>)>,
    /// The last render that failed, not tried again for the same key.
    failed: Option<Key>,
    /// Whether the texture last drawn was the sized render.
    sharp: bool,
}

impl Held {
    /// The pixel size of the sized render when it is the one on screen.
    pub(super) fn sharp_size(&self) -> Option<[usize; 2]> {
        self.sized
            .as_ref()
            .filter(|_| self.sharp)
            .map(|(_, texture)| texture.size())
    }

    fn texture(
        &mut self,
        ctx: &egui::Context,
        cover: &Cover<'_>,
        size: [u32; 2],
        budget: &engine::ByteBudget,
    ) -> Option<&egui::TextureHandle> {
        if let Some((key, receiver)) = self.pending.take() {
            match receiver.try_recv() {
                Ok(Some(image)) => {
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(
                        [image.width as usize, image.height as usize],
                        &image.rgba,
                    );
                    let texture = ctx.load_texture(
                        "export.cover.sized",
                        pixels,
                        egui::TextureOptions::LINEAR,
                    );
                    self.sized = Some((key, texture));
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.failed = Some(key),
                Err(mpsc::TryRecvError::Empty) => self.pending = Some((key, receiver)),
            }
        }
        let key = Key {
            project: cover.project.clone(),
            tick: cover.tick,
            size,
        };
        let current = self.sized.as_ref().is_some_and(|(held, _)| held.same(&key));
        let waiting = self
            .pending
            .as_ref()
            .is_some_and(|(held, _)| held.same(&key));
        let failed = self.failed.as_ref().is_some_and(|held| held.same(&key));
        if !current && !waiting && !failed {
            let (sender, receiver) = wake::channel();
            let (project, tick, budget) = (cover.project.clone(), cover.tick, budget.clone());
            let corrections = (cover.corrections)();
            wake::spawn(move || {
                let hooks = engine::ByteBudgetHooks::new(&budget, None);
                let image = engine::cover::render(&project, tick, size, &hooks, &corrections).ok();
                let _ = sender.send(image);
            });
            self.pending = Some((key, receiver));
        }
        self.sharp = current;
        if current {
            return self.sized.as_ref().map(|(_, texture)| texture);
        }
        self.thumbnail_texture(ctx, cover.thumbnail)
    }

    fn thumbnail_texture(
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
            self.thumbnail = None;
            return None;
        };
        if self
            .thumbnail
            .as_ref()
            .is_none_or(|(held, _)| held != thumbnail)
        {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([*width as usize, *height as usize], rgba);
            let texture = ctx.load_texture("export.cover", image, egui::TextureOptions::LINEAR);
            self.thumbnail = Some((thumbnail.clone(), texture));
        }
        self.thumbnail.as_ref().map(|(_, texture)| texture)
    }
}

/// How the cover column is laid out and what it says.
pub(super) struct View<'a> {
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
    /// The byte budget the sized render draws on.
    pub budget: &'a engine::ByteBudget,
}

/// Draw the project cover over the canvas ratio with the Edit cover chip at its top-left corner
/// [measured: frame 105340, 8 px in from the cover's top and left edges].
pub(super) fn show(
    ui: &mut egui::Ui,
    held: &mut Held,
    cover: Cover<'_>,
    view: View<'_>,
    result: &mut DialogUi,
) {
    let tr = |key| i18n::text(view.locale, key);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(WIDTH, (WIDTH * view.aspect).min(view.height)),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 4.0, theme::window(view.dark));
    // The frame fills the column's width unless the form leaves too little height; the sized
    // render has exactly the physical pixels it is drawn over.
    let shown_width = rect.width().min(rect.height() / view.aspect);
    let scale = ui.ctx().pixels_per_point();
    let size = [
        (shown_width * scale).round().max(1.0) as u32,
        (shown_width * view.aspect * scale).round().max(1.0) as u32,
    ];
    if let Some(texture) = held.texture(ui.ctx(), &cover, size, view.budget) {
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

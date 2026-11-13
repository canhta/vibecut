//! Crop dialog over the shared crop control and the item crop patch.

use super::State;
use crate::{
    Result,
    editor::Port,
    egui,
    i18n::{self, Locale, Text},
    kit as theme,
    kit::Icon,
    services::{crop::control, lifecycle::EditorSession},
};
use project::ItemId;

/// Aspect shapes the dialog offers, in the reference order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropAspect {
    Custom,
    Original,
    NineSixteen,
    Square,
    ThreeFour,
    SixteenNine,
    FourThree,
    TwoOne,
}

impl CropAspect {
    const ALL: [Self; 8] = [
        Self::Custom,
        Self::Original,
        Self::NineSixteen,
        Self::Square,
        Self::ThreeFour,
        Self::SixteenNine,
        Self::FourThree,
        Self::TwoOne,
    ];

    fn label(self) -> Text {
        match self {
            Self::Custom => Text::CropCustom,
            Self::Original => Text::CropOriginal,
            Self::NineSixteen => Text::CropAspect9x16,
            Self::Square => Text::CropAspect1x1,
            Self::ThreeFour => Text::CropAspect3x4,
            Self::SixteenNine => Text::CropAspect16x9,
            Self::FourThree => Text::CropAspect4x3,
            Self::TwoOne => Text::CropAspect2x1,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Custom => "crop.aspect.custom",
            Self::Original => "crop.aspect.original",
            Self::NineSixteen => "crop.aspect.9x16",
            Self::Square => "crop.aspect.1x1",
            Self::ThreeFour => "crop.aspect.3x4",
            Self::SixteenNine => "crop.aspect.16x9",
            Self::FourThree => "crop.aspect.4x3",
            Self::TwoOne => "crop.aspect.2x1",
        }
    }

    /// Width divided by height in output pixels; `None` for Custom, which keeps
    /// the current box.
    fn ratio(self) -> Option<f64> {
        Some(match self {
            Self::Custom => return None,
            Self::Original => return None,
            Self::NineSixteen => 9.0 / 16.0,
            Self::Square => 1.0,
            Self::ThreeFour => 3.0 / 4.0,
            Self::SixteenNine => 16.0 / 9.0,
            Self::FourThree => 4.0 / 3.0,
            Self::TwoOne => 2.0,
        })
    }

    /// The shape lock in source pixels for the control; Custom resizes freely
    /// and Original keeps the whole source frame.
    fn lock(self, aspect: f64) -> Option<f64> {
        match self {
            Self::Custom => None,
            Self::Original => Some(aspect),
            _ => self.ratio(),
        }
    }

    /// Largest centred rectangle of this shape inside a source of `aspect`.
    /// Original keeps the whole frame; Custom leaves the box untouched.
    fn crop(self, aspect: f64) -> Option<project::Crop> {
        if self == Self::Custom {
            return None;
        }
        if self == Self::Original {
            return Some(project::Crop::default());
        }
        let ratio = self.ratio()? / aspect;
        let (width, height) = if ratio >= 1.0 {
            (1.0, 1.0 / ratio)
        } else {
            (ratio, 1.0)
        };
        Some(project::Crop {
            x: (1.0 - width) / 2.0,
            y: (1.0 - height) / 2.0,
            width,
            height,
        })
    }

    /// The shape a rectangle already has, so reopening a crop shows its ratio.
    /// Only the shape matters, not where the box sits.
    fn of(crop: project::Crop, aspect: f64) -> Self {
        let actual = crop.width / crop.height * aspect;
        Self::ALL
            .into_iter()
            .find(|choice| {
                choice
                    .lock(aspect)
                    .is_some_and(|lock| (actual - lock).abs() / lock <= 1e-3)
            })
            .unwrap_or(Self::Custom)
    }
}

/// Read-only dialog state for automation; the project itself stays untouched
/// until Apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropInspection {
    pub item: ItemId,
    pub aspect: CropAspect,
    pub crop: project::Crop,
}

/// The clip the dialog edits and the source shape its ratios resolve against.
struct Target {
    item: ItemId,
    aspect: f64,
    original: Option<project::Crop>,
}

/// The single selected video or image clip, with its source pixel aspect.
fn target(session: &EditorSession) -> Option<Target> {
    let id = crate::editor::context::single(session.selection())?;
    let project = session.project();
    let data = project.data();
    let item = data
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == id)?;
    let project::ItemParams::Video(params) = &item.content else {
        return None;
    };
    let asset = data
        .assets
        .iter()
        .find(|asset| Some(asset.id()) == item.source_asset)?;
    let video = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())?;
    if video.width == 0 || video.height == 0 {
        return None;
    }
    let aspect = f64::from(video.width) / f64::from(video.height);
    aspect.is_finite().then_some(Target {
        item: id,
        aspect,
        original: params.crop,
    })
}

/// Transient dialog over one clip's existing crop.
pub(crate) struct Dialog {
    item: ItemId,
    original: Option<project::Crop>,
    crop: project::Crop,
    aspect: CropAspect,
}

impl Dialog {
    /// Open on the clip's current crop, or the full frame when it has none.
    fn new(target: Target) -> Self {
        let crop = target.original.unwrap_or_default();
        Self {
            item: target.item,
            original: target.original,
            crop,
            aspect: CropAspect::of(crop, target.aspect),
        }
    }

    pub(crate) fn inspect(&self) -> CropInspection {
        CropInspection {
            item: self.item,
            aspect: self.aspect,
            crop: self.crop,
        }
    }

    /// The patch Apply writes: absent when the box is the whole frame.
    fn patch(&self) -> Option<project::Crop> {
        (self.crop != project::Crop::default()).then_some(self.crop)
    }
}

/// Open the dialog for the selected clip; a non-croppable target refuses.
pub(crate) fn open(state: &mut State, port: &Port<'_>) -> Result<()> {
    let target = target(port.session).ok_or(crate::Error::Unsupported("crop target"))?;
    state.crop = Some(Dialog::new(target));
    Ok(())
}

/// Whether the session's selection has a source frame the dialog can show.
pub(crate) fn available(session: &EditorSession) -> bool {
    target(session).is_some()
}

enum Action {
    Apply,
}

#[derive(Default)]
struct Output {
    widgets: Vec<(&'static str, egui::Response)>,
    action: Option<Action>,
    close: bool,
}

/// Draw the open dialog over the frame's context. Every gesture stays in the
/// dialog; only Apply reaches the engine, as one undo step.
pub(crate) fn show(state: &mut State, port: &mut Port<'_>, ctx: &egui::Context) {
    let Some(mut dialog) = state.crop.take() else {
        return;
    };
    let Some(target) = target(port.session) else {
        return;
    };
    if target.item != dialog.item {
        return;
    }
    let texture = state.texture.handle().cloned();
    let output = dialog.ui(
        ctx,
        port.locale(),
        port.dark(),
        target.aspect,
        texture.as_ref(),
    );
    for (id, response) in output.widgets {
        port.widget(id, None, None, &response);
    }
    if output.close {
        return;
    }
    match output.action {
        Some(Action::Apply) => {
            if dialog.apply(port).is_err() {
                state.crop = Some(dialog);
            }
        }
        None => state.crop = Some(dialog),
    }
}

impl Dialog {
    /// One documented crop patch, or no history entry when the box is unchanged. A
    /// refusal reaches the error line through the port.
    fn apply(&self, port: &mut Port<'_>) -> Result<()> {
        let patch = self.patch();
        if patch == self.original {
            return Ok(());
        }
        port.apply_result(engine::Command::SetItemField {
            item: self.item,
            patch: engine::ItemParamPatch::Crop(patch),
        })?;
        Ok(())
    }

    fn ui(
        &mut self,
        ctx: &egui::Context,
        locale: Locale,
        dark: bool,
        source_aspect: f64,
        texture: Option<&egui::TextureHandle>,
    ) -> Output {
        let mut output = Output::default();
        let tr = |key| i18n::text(locale, key);
        let frame = egui::Frame::window(&ctx.global_style())
            .corner_radius(12)
            .inner_margin(24);
        let margin = frame.total_margin();
        let modal = egui::Modal::new(egui::Id::unique("crop.dialog"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::proportional(14.0));
                ui.set_width(680.0_f32.min(ctx.content_rect().width() - 48.0) - margin.sum().x);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr(Text::CropTitle)).strong().size(18.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close = crate::kit::icon_button(
                            ui,
                            "crop.close",
                            Icon::X,
                            tr(Text::CloseScenes),
                            false,
                        );
                        if close.clicked() {
                            output.close = true;
                        }
                        output.widgets.push(("crop.close", close));
                    });
                });
                ui.add_space(16.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(200.0);
                        ui.label(egui::RichText::new(tr(Text::CropAspectRatio)).strong());
                        ui.add_space(8.0);
                        let combo = egui::ComboBox::from_id_salt(egui::Id::unique("crop.aspect"))
                            .width(200.0)
                            .height(300.0)
                            .selected_text(tr(self.aspect.label()))
                            .show_ui(ui, |ui| {
                                for aspect in CropAspect::ALL {
                                    let response = ui.selectable_label(
                                        self.aspect == aspect,
                                        tr(aspect.label()),
                                    );
                                    if response.clicked() {
                                        if let Some(crop) = aspect.crop(source_aspect) {
                                            self.crop = crop;
                                        }
                                        self.aspect = aspect;
                                    }
                                    output.widgets.push((aspect.id(), response));
                                }
                            });
                        output.widgets.push(("crop.aspect", combo.response));
                    });
                    ui.add_space(24.0);
                    ui.vertical(|ui| {
                        ui.set_width(430.0);
                        match control(
                            ui,
                            "crop.preview",
                            &mut self.crop,
                            source_aspect as f32,
                            self.aspect.lock(source_aspect),
                            texture,
                            tr(Text::CropPreview),
                        ) {
                            Ok(response) => {
                                output.widgets.push(("crop.preview", response));
                            }
                            Err(error) => {
                                ui.colored_label(theme::error_text(dark), error.to_string());
                            }
                        }
                    });
                });
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let apply = ui.add(
                            egui::Button::new(
                                egui::RichText::new(tr(Text::CropApply))
                                    .color(theme::on_accent(dark)),
                            )
                            .fill(theme::accent(dark))
                            .min_size(egui::vec2(90.0, 34.0)),
                        );
                        if apply.clicked() {
                            output.action = Some(Action::Apply);
                        }
                        output.widgets.push(("crop.apply", apply));
                        ui.add_space(8.0);
                        let reset =
                            ui.add_sized([90.0, 34.0], egui::Button::new(tr(Text::CropReset)));
                        if reset.clicked() {
                            self.crop = project::Crop::default();
                            self.aspect = CropAspect::Original;
                        }
                        output.widgets.push(("crop.reset", reset));
                    });
                });
            });
        if modal.should_close() {
            output.close = true;
        }
        output
    }
}

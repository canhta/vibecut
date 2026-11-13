//! Renders output settings over the portable export contract.
use super::dialog::{DialogUi, ExportOptions, RangeChoice, SceneChoice};
use crate::i18n;
use crate::{
    egui,
    i18n::{Locale, Text},
};

const RESOLUTIONS: [(u32, &str, &str); 7] = [
    (360, "360P", "export.resolution.360"),
    (480, "480P", "export.resolution.480"),
    (720, "720P", "export.resolution.720"),
    (1080, "1080P", "export.resolution.1080"),
    (1440, "2K", "export.resolution.2k"),
    (2160, "4K", "export.resolution.4k"),
    (4320, "8K", "export.resolution.8k"),
];
fn resolution_size(project: &project::Project, height: u32) -> [u32; 2] {
    let canvas = &project.data().canvas;
    let (long, short) = if canvas.width >= canvas.height {
        (canvas.width, canvas.height)
    } else {
        (canvas.height, canvas.width)
    };
    let out_long = ((u64::from(height) * u64::from(long) + u64::from(short))
        / (2 * u64::from(short).max(1))
        * 2)
    .min(u64::from(u32::MAX)) as u32;
    if canvas.width >= canvas.height {
        [out_long, height]
    } else {
        [height, out_long]
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    options: &mut ExportOptions,
    project: &project::Project,
    selection: Option<time::TimeRange>,
    result: &mut DialogUi,
) {
    let tr = |key| i18n::text(locale, key);
    let spacious = ui.ctx().content_rect().height() >= 900.0;
    let field_width = (ui.available_width() - 131.0).clamp(96.0, 280.0);
    let identity_width = (ui.available_width() - 100.0).max(96.0);
    let audio_only = options.format.audio_only();
    egui::Grid::new("export.identity")
        .min_col_width(84.0)
        .spacing([16.0, if spacious { 24.0 } else { 12.0 }])
        .show(ui, |ui| {
            for (key, value) in [
                (Text::ExportName, project.data().name.as_str()),
                (Text::ExportDestination, tr(Text::ExportChooseAfter)),
            ] {
                ui.label(tr(key));
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .corner_radius(2)
                    .inner_margin(egui::Margin {
                        left: 4,
                        right: 4,
                        top: 2,
                        bottom: 3,
                    })
                    .show(ui, |ui| {
                        // A top-down frame is left-aligned; `add_sized` would centre the text.
                        ui.set_width(identity_width - 8.0);
                        ui.set_min_height(14.0);
                        ui.add(egui::Label::new(value).truncate());
                    });
                ui.end_row();
            }
        });
    ui.add_space(if spacious { 44.0 } else { 12.0 });
    let section = if audio_only { Text::Audio } else { Text::Video };
    ui.label(egui::RichText::new(tr(section)).strong());
    ui.add_space(if spacious { 26.0 } else { 8.0 });
    egui::Grid::new("export.output")
        .num_columns(2)
        .min_col_width(115.0)
        .spacing([16.0, if spacious { 26.0 } else { 16.0 }])
        .show(ui, |ui| {
            if !audio_only {
                let row_label = ui.label(tr(Text::ExportResolution));
                let label = options.output.resolution.map_or_else(
                    || tr(Text::ExportProjectCanvas).to_owned(),
                    |size| {
                        RESOLUTIONS
                            .iter()
                            .find(|(height, _, _)| resolution_size(project, *height) == size)
                            .map_or_else(
                                || {
                                    let [width, height] = size;
                                    format!("{width} × {height}")
                                },
                                |(_, label, _)| (*label).to_owned(),
                            )
                    },
                );
                let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.resolution"))
                    .width(field_width)
                    .height(340.0)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        ui.spacing_mut().interact_size.y = 36.0;
                        let r = ui.selectable_value(
                            &mut options.output.resolution,
                            None,
                            tr(Text::ExportProjectCanvas),
                        );
                        result.widgets.push(("export.resolution.project", r));
                        for (h, label, id) in RESOLUTIONS {
                            let size = resolution_size(project, h);
                            let r = ui.selectable_value(
                                &mut options.output.resolution,
                                Some(size),
                                label,
                            );
                            result.widgets.push((id, r));
                        }
                    });
                result.widgets.push((
                    "export.resolution",
                    combo.response.labelled_by(row_label.id),
                ));
                ui.end_row();
                let row_label = ui.label(tr(Text::ExportBitRate));
                ui.horizontal(|ui| {
                    let mut custom = options.output.bit_rate.is_some();
                    let r = ui.add_enabled(
                        options.format != super::ExportFormat::Gif,
                        egui::Checkbox::new(&mut custom, tr(Text::ExportCustom)),
                    );
                    if r.changed() {
                        options.output.bit_rate = custom.then_some(options.quality.value().1);
                    }
                    result
                        .widgets
                        .push(("export.bitrate.custom", r.labelled_by(row_label.id)));
                    if let Some(rate) = &mut options.output.bit_rate {
                        let r = ui.add(
                            egui::DragValue::new(rate)
                                .range(1_000..=1_000_000_000)
                                .speed(1000)
                                .suffix(" b/s"),
                        );
                        result
                            .widgets
                            .push(("export.bitrate", r.labelled_by(row_label.id)));
                    }
                });
                ui.end_row();
                let row_label = ui.label(tr(Text::ExportCodec));
                let codec_label = |codec| match codec {
                    render::VideoCodec::Automatic => tr(Text::ExportAutomatic),
                    render::VideoCodec::H264 => "H.264",
                    render::VideoCodec::Vp9 => "VP9",
                };
                let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.codec"))
                    .width(field_width)
                    .selected_text(codec_label(options.output.codec))
                    .show_ui(ui, |ui| {
                        for (codec, id) in [
                            (render::VideoCodec::Automatic, "export.codec.auto"),
                            (render::VideoCodec::H264, "export.codec.h264"),
                            (render::VideoCodec::Vp9, "export.codec.vp9"),
                        ] {
                            let allowed = codec.fits(options.format.container());
                            let r = ui.add_enabled(
                                allowed,
                                egui::Button::selectable(
                                    options.output.codec == codec,
                                    codec_label(codec),
                                ),
                            );
                            if r.clicked() {
                                options.output.codec = codec;
                            }
                            result.widgets.push((id, r));
                        }
                    });
                result
                    .widgets
                    .push(("export.codec", combo.response.labelled_by(row_label.id)));
                ui.end_row();
            }
            let row_label = ui.label(tr(Text::ExportFormat));
            let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.format"))
                .width(field_width)
                .selected_text(tr(options.format.label()))
                .show_ui(ui, |ui| {
                    for (format, id) in [
                        (super::ExportFormat::Mp4, "export.format.mp4"),
                        (super::ExportFormat::Mov, "export.format.mov"),
                        (super::ExportFormat::Webm, "export.format.webm"),
                        (super::ExportFormat::M4a, "export.format.m4a"),
                        (super::ExportFormat::AudioWebm, "export.format.audio_webm"),
                        (super::ExportFormat::Gif, "export.format.gif"),
                    ] {
                        let available = if format.audio_only() {
                            media::audio_export_container_available(format.container())
                        } else {
                            media::export_container_available(format.container())
                        };
                        if !available {
                            continue;
                        }
                        let r =
                            ui.selectable_value(&mut options.format, format, tr(format.label()));
                        if r.changed() {
                            options.output.codec = render::VideoCodec::Automatic;
                            options.output.profile = media::H264Profile::Automatic;
                            if format == super::ExportFormat::Gif {
                                options.output.bit_rate = None;
                            }
                        }
                        result.widgets.push((id, r));
                    }
                });
            result
                .widgets
                .push(("export.format", combo.response.labelled_by(row_label.id)));
            ui.end_row();
            if !audio_only {
                let row_label = ui.label(tr(Text::ExportFrameRate));
                let label = options.output.fps.map_or_else(
                    || tr(Text::ExportProjectCanvas).to_owned(),
                    |fps| format!("{}", fps.as_f64()),
                );
                let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.fps"))
                    .width(field_width)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut options.output.fps,
                            None,
                            tr(Text::ExportProjectCanvas),
                        );
                        for (fps, id) in [
                            (time::FrameRate::FPS_24, "export.fps.24"),
                            (time::FrameRate::FPS_25, "export.fps.25"),
                            (time::FrameRate::FPS_30, "export.fps.30"),
                            (time::FrameRate::FPS_50, "export.fps.50"),
                            (time::FrameRate::FPS_60, "export.fps.60"),
                        ] {
                            let r = ui.selectable_value(
                                &mut options.output.fps,
                                Some(fps),
                                fps.num().to_string(),
                            );
                            result.widgets.push((id, r));
                        }
                    });
                result
                    .widgets
                    .push(("export.fps", combo.response.labelled_by(row_label.id)));
                ui.end_row();
                let row_label = ui.label(tr(Text::ExportQuality));
                let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.quality"))
                    .width(field_width)
                    .selected_text(tr(options.quality.label()))
                    .show_ui(ui, |ui| {
                        for (quality, id) in [
                            (super::ExportQuality::Low, "export.quality.low"),
                            (super::ExportQuality::Medium, "export.quality.medium"),
                            (super::ExportQuality::High, "export.quality.high"),
                            (super::ExportQuality::VeryHigh, "export.quality.very_high"),
                        ] {
                            let r = ui.selectable_value(
                                &mut options.quality,
                                quality,
                                tr(quality.label()),
                            );
                            result.widgets.push((id, r));
                        }
                    });
                result
                    .widgets
                    .push(("export.quality", combo.response.labelled_by(row_label.id)));
                ui.end_row();
                let row_label = ui.label(tr(Text::ExportProfile));
                let label = |profile| match profile {
                    media::H264Profile::Automatic => tr(Text::ExportAutomatic),
                    media::H264Profile::Main => "Main",
                    media::H264Profile::High => "High",
                };
                let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.profile"))
                    .width(field_width)
                    .selected_text(label(options.output.profile))
                    .show_ui(ui, |ui| {
                        for profile in [
                            media::H264Profile::Automatic,
                            media::H264Profile::Main,
                            media::H264Profile::High,
                        ] {
                            ui.add_enabled_ui(
                                render::OutputOptions::profile_fits(
                                    profile,
                                    options.format.container(),
                                ),
                                |ui| {
                                    ui.selectable_value(
                                        &mut options.output.profile,
                                        profile,
                                        label(profile),
                                    );
                                },
                            );
                        }
                    });
                result
                    .widgets
                    .push(("export.profile", combo.response.labelled_by(row_label.id)));
                ui.end_row();
            }
            if options.format == super::ExportFormat::Gif {
                gif_rows(ui, locale, options, field_width, result);
            }
            let row_label = ui.label(tr(Text::ExportScene));
            let previous_scene = options.output.scene;
            let label = match options.output.scene {
                SceneChoice::Active => tr(Text::ExportActiveScene),
                SceneChoice::All => tr(Text::ExportAllScenes),
                SceneChoice::Selected { scene } => project
                    .data()
                    .scenes
                    .get(scene)
                    .map_or(tr(Text::ExportMissingScene), |scene| scene.name.as_str()),
            };
            let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.scene"))
                .width(field_width)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (value, label, id) in [
                        (
                            SceneChoice::Active,
                            tr(Text::ExportActiveScene),
                            "export.scene.active",
                        ),
                        (
                            SceneChoice::All,
                            tr(Text::ExportAllScenes),
                            "export.scene.all",
                        ),
                    ] {
                        let r = ui.selectable_value(&mut options.output.scene, value, label);
                        result.widgets.push((id, r));
                    }
                    for scene in project.data().scenes.iter() {
                        let r = ui.selectable_value(
                            &mut options.output.scene,
                            SceneChoice::Selected { scene: scene.id },
                            scene.name.as_str(),
                        );
                        if r.clicked() {
                            options.output.range = RangeChoice::Full;
                        }
                    }
                });
            result
                .widgets
                .push(("export.scene", combo.response.labelled_by(row_label.id)));
            if previous_scene != options.output.scene {
                options.output.range = RangeChoice::Full;
            }
            ui.end_row();
            let row_label = ui.label(tr(Text::ExportRange));
            let label = match options.output.range {
                RangeChoice::Full => Text::ExportFullRange,
                RangeChoice::InOut { .. } => Text::ExportInOut,
                RangeChoice::Selected { .. } => Text::ExportSelection,
            };
            let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.range"))
                .width(field_width)
                .selected_text(tr(label))
                .show_ui(ui, |ui| {
                    let r = ui.selectable_value(
                        &mut options.output.range,
                        RangeChoice::Full,
                        tr(Text::ExportFullRange),
                    );
                    result.widgets.push(("export.range.full", r));
                    let r = ui.selectable_label(
                        matches!(options.output.range, RangeChoice::InOut { .. }),
                        tr(Text::ExportInOut),
                    );
                    if r.clicked() && !matches!(options.output.range, RangeChoice::InOut { .. }) {
                        options.output.range = RangeChoice::InOut {
                            start: time::Tick::ZERO,
                            end: match options.output.scene {
                                SceneChoice::Selected { scene } => project
                                    .data()
                                    .scenes
                                    .get(scene)
                                    .map_or(time::Tick::ZERO, |scene| scene.duration()),
                                _ => project.duration(),
                            },
                        };
                    }
                    result.widgets.push(("export.range.in_out", r));
                    let enabled =
                        selection.is_some() && options.output.scene == SceneChoice::Active;
                    let r = ui.add_enabled(
                        enabled,
                        egui::Button::selectable(
                            matches!(options.output.range, RangeChoice::Selected { .. }),
                            tr(Text::ExportSelection),
                        ),
                    );
                    if r.clicked()
                        && let Some(range) = selection
                    {
                        options.output.range = RangeChoice::Selected {
                            start: range.start(),
                            end: range.end(),
                        };
                    }
                    result.widgets.push(("export.range.selected", r));
                });
            result
                .widgets
                .push(("export.range", combo.response.labelled_by(row_label.id)));
            ui.end_row();
            if let RangeChoice::InOut { start, end } = &mut options.output.range {
                ui.label(tr(Text::ExportInOut));
                ui.horizontal(|ui| {
                    for (tick, id, label) in [
                        (start, "export.range.in", tr(Text::ExportIn)),
                        (end, "export.range.out", tr(Text::ExportOut)),
                    ] {
                        let mut frame = project.data().canvas.fps.frame_at(*tick).unwrap_or(0);
                        let r = ui.add(
                            egui::DragValue::new(&mut frame)
                                .prefix(format!("{label} "))
                                .suffix(" f")
                                .speed(1),
                        );
                        if r.changed()
                            && let Ok(value) = project.data().canvas.fps.tick_of(frame)
                        {
                            *tick = value;
                        }
                        result.widgets.push((id, r));
                    }
                });
                ui.end_row();
            }
        });
    let custom = egui::CollapsingHeader::new(tr(Text::ExportCustomSize))
        .id_salt("export.custom.size")
        .show(ui, |ui| {
            let mut size = options
                .output
                .resolution
                .unwrap_or([project.data().canvas.width, project.data().canvas.height]);
            ui.horizontal(|ui| {
                let width = ui.add(
                    egui::DragValue::new(&mut size[0])
                        .range(2..=32768)
                        .speed(2)
                        .prefix(format!("{} ", tr(Text::ExportWidth))),
                );
                let height = ui.add(
                    egui::DragValue::new(&mut size[1])
                        .range(2..=32768)
                        .speed(2)
                        .prefix(format!("{} ", tr(Text::ExportHeight))),
                );
                if width.changed() || height.changed() {
                    options.output.resolution = Some(size);
                }
                result.widgets.push(("export.width", width));
                result.widgets.push(("export.height", height));
            });
        });
    result
        .widgets
        .push(("export.custom.size", custom.header_response));
    ui.add_space(12.0);
    let r = ui.checkbox(&mut options.include_audio, tr(Text::IncludeAudio));
    result.widgets.push(("export.audio.include", r));
    if !valid(options, project) {
        ui.colored_label(ui.visuals().error_fg_color, tr(Text::ExportInvalidRange));
    }
}

/// Colors and dithering for GIF output, as two more grid rows.
fn gif_rows(
    ui: &mut egui::Ui,
    locale: Locale,
    options: &mut ExportOptions,
    field_width: f32,
    result: &mut DialogUi,
) {
    let tr = |key| i18n::text(locale, key);
    let row_label = ui.label(tr(Text::ExportGifColors));
    let r = ui.add(
        egui::DragValue::new(&mut options.output.gif.max_colors)
            .range(media::GIF_MIN_COLORS..=media::GIF_MAX_COLORS)
            .speed(1),
    );
    result
        .widgets
        .push(("export.gif.colors", r.labelled_by(row_label.id)));
    ui.end_row();
    let row_label = ui.label(tr(Text::ExportGifDither));
    let label = |dither| match dither {
        media::GifDither::None => tr(Text::ExportGifDitherNone),
        media::GifDither::Bayer => tr(Text::ExportGifDitherBayer),
    };
    let combo = egui::ComboBox::from_id_salt(egui::IdSalt::new("export.gif.dither"))
        .width(field_width)
        .selected_text(label(options.output.gif.dither))
        .show_ui(ui, |ui| {
            for (dither, id) in [
                (media::GifDither::None, "export.gif.dither.none"),
                (media::GifDither::Bayer, "export.gif.dither.bayer"),
            ] {
                let r = ui.selectable_value(&mut options.output.gif.dither, dither, label(dither));
                result.widgets.push((id, r));
            }
        });
    result.widgets.push((
        "export.gif.dither",
        combo.response.labelled_by(row_label.id),
    ));
    ui.end_row();
}

pub(super) fn valid(options: &ExportOptions, project: &project::Project) -> bool {
    let mut settings = options.settings(project);
    options.output.ranges(project).is_ok()
        && options.output.apply(&mut settings).is_ok()
        && options.output.check_gif(project, &settings).is_ok()
}

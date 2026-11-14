//! Renders output settings over the portable export contract.
use super::dialog::{DialogUi, ExportFormat, ExportOptions, RangeChoice, SceneChoice};
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

/// Which tickable section the saved format selects. One export job writes one file, so
/// the three sections are exclusive: ticking one unticks the others.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Video, with its audio track when Audio is ticked, or a PNG sequence.
    Video,
    /// Audio only: Video unticked, Audio ticked.
    Audio,
    Gif,
}
impl Section {
    fn of(format: ExportFormat) -> Self {
        if format.is_gif() {
            Self::Gif
        } else if format.audio_only() {
            Self::Audio
        } else {
            Self::Video
        }
    }
}

const GIF_RESOLUTIONS: [(u32, &str, &str); 3] = [
    (240, "240P", "export.gif.resolution.240"),
    (320, "320P", "export.gif.resolution.320"),
    (640, "640P", "export.gif.resolution.640"),
];
/// The Custom bit rate's starting value and ceiling, in Kbps, as the reference's field.
const CUSTOM_KBPS_DEFAULT: usize = 12_000;
const CUSTOM_KBPS_MAX: u32 = 50_000;

fn last_format_key(audio: bool) -> egui::Id {
    egui::Id::unique(if audio {
        "export.last_audio_format"
    } else {
        "export.last_video_format"
    })
}

/// The format a section last held, so unticking and re-ticking returns to the choice.
fn last_format(ui: &egui::Ui, audio: bool) -> ExportFormat {
    ui.data(|data| data.get_temp::<ExportFormat>(last_format_key(audio)))
        .unwrap_or(if audio {
            ExportFormat::M4a
        } else {
            ExportFormat::Mp4
        })
}

/// Switches the saved format, resetting the values every output accepts.
fn select_format(options: &mut ExportOptions, format: ExportFormat) {
    options.format = format;
    options.output.codec = render::VideoCodec::Automatic;
    options.output.profile = media::H264Profile::Automatic;
    if format.unused_encoder_reason().is_some() {
        options.output.bit_rate = None;
    }
    if format.is_sequence() && options.output.scene == SceneChoice::All {
        options.output.scene = SceneChoice::Active;
    }
}

/// Moves to `to`. GIF carries its own resolution list, so entering or leaving it resets the
/// shared size (240P entering, the project canvas leaving).
fn select_section(
    ui: &egui::Ui,
    options: &mut ExportOptions,
    project: &project::Project,
    to: Section,
) {
    let from = Section::of(options.format);
    match to {
        Section::Video => select_format(options, last_format(ui, false)),
        Section::Audio => {
            select_format(options, last_format(ui, true));
            options.include_audio = true;
        }
        Section::Gif => {
            select_format(options, ExportFormat::Gif);
            options.output.resolution = Some(resolution_size(project, GIF_RESOLUTIONS[0].0));
        }
    }
    if from == Section::Gif && to != Section::Gif {
        options.output.resolution = None;
    }
}

/// A section's tick box as a bold header. A disabled one names why on hover.
fn section_header(
    ui: &mut egui::Ui,
    locale: Locale,
    ticked: bool,
    label: Text,
    refused: Option<Text>,
) -> (bool, egui::Response) {
    let mut value = ticked;
    let response = gated(ui, locale, refused, |ui| {
        ui.checkbox(
            &mut value,
            egui::RichText::new(i18n::text(locale, label)).strong(),
        )
    });
    (value != ticked, response)
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
    if options.format.audio_only() {
        ui.data_mut(|data| data.insert_temp(last_format_key(true), options.format));
    } else if !options.format.is_gif() {
        ui.data_mut(|data| data.insert_temp(last_format_key(false), options.format));
    }
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
    // Video: ticked unless Audio or GIF was chosen. Its block is greyed, not hidden, otherwise.
    let section = Section::of(options.format);
    let video_alone =
        section == Section::Video && (!options.include_audio || options.format.is_sequence());
    let (changed, r) = section_header(
        ui,
        locale,
        section == Section::Video,
        Text::Video,
        video_alone.then_some(Text::ExportKeepOne),
    );
    result.widgets.push(("export.video.include", r));
    if changed {
        let to = if section == Section::Video {
            Section::Audio
        } else {
            Section::Video
        };
        select_section(ui, options, project, to);
    }
    ui.add_space(if spacious { 26.0 } else { 8.0 });
    let section = Section::of(options.format);
    // The reason the video rows are off for the current section, if they are.
    let video_off = match section {
        Section::Video => None,
        Section::Audio => Some(Text::ExportNotForAudio),
        Section::Gif => Some(Text::ExportNotForGif),
    };
    egui::Grid::new("export.output")
        .num_columns(2)
        .min_col_width(115.0)
        .spacing([16.0, if spacious { 26.0 } else { 16.0 }])
        .show(ui, |ui| {
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
            let combo = gated(ui, locale, video_off, |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.resolution"))
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
                    })
                    .response
            });
            result
                .widgets
                .push(("export.resolution", combo.labelled_by(row_label.id)));
            ui.end_row();
            let row_label = ui
                .horizontal(|ui| {
                    let label = ui.label(tr(Text::ExportBitRate));
                    ui.weak("?").on_hover_text(tr(Text::ExportBitRateHint));
                    label
                })
                .inner;
            ui.vertical(|ui| {
                let reason = options.format.unused_encoder_reason();
                let custom = options.output.bit_rate.is_some();
                let selected = if custom {
                    Text::ExportCustom
                } else {
                    options.quality.label()
                };
                let combo = gated(ui, locale, reason, |ui| {
                    egui::ComboBox::from_id_salt(egui::IdSalt::new("export.bitrate"))
                        .width(field_width)
                        .selected_text(tr(selected))
                        .show_ui(ui, |ui| {
                            for (quality, id) in [
                                (super::ExportQuality::Lower, "export.bitrate.lower"),
                                (
                                    super::ExportQuality::Recommended,
                                    "export.bitrate.recommended",
                                ),
                                (super::ExportQuality::Higher, "export.bitrate.higher"),
                            ] {
                                let r = ui.selectable_label(
                                    !custom && options.quality == quality,
                                    tr(quality.label()),
                                );
                                if r.clicked() {
                                    options.quality = quality;
                                    options.output.bit_rate = None;
                                }
                                result.widgets.push((id, r));
                            }
                            let r = ui.selectable_label(custom, tr(Text::ExportCustom));
                            if r.clicked() && !custom {
                                options.output.bit_rate = Some(CUSTOM_KBPS_DEFAULT * 1000);
                            }
                            result.widgets.push(("export.bitrate.custom", r));
                        })
                        .response
                });
                result
                    .widgets
                    .push(("export.bitrate", combo.labelled_by(row_label.id)));
                if let Some(rate) = &mut options.output.bit_rate {
                    let mut kbps = (*rate / 1000).clamp(1, CUSTOM_KBPS_MAX as usize) as u32;
                    let r = gated(ui, locale, reason, |ui| {
                        ui.add(
                            egui::DragValue::new(&mut kbps)
                                .range(1..=CUSTOM_KBPS_MAX)
                                .speed(100)
                                .suffix(" Kbps"),
                        )
                    });
                    if r.changed() {
                        *rate = kbps as usize * 1000;
                    }
                    result
                        .widgets
                        .push(("export.bitrate.kbps", r.labelled_by(row_label.id)));
                }
            });
            ui.end_row();
            let row_label = ui.label(tr(Text::ExportCodec));
            let codec_label = |codec| match codec {
                render::VideoCodec::Automatic => tr(Text::ExportAutomatic),
                render::VideoCodec::H264 => "H.264",
                render::VideoCodec::Vp9 => "VP9",
            };
            let combo = gated(ui, locale, options.format.unused_encoder_reason(), |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.codec"))
                    .width(field_width)
                    .selected_text(codec_label(options.output.codec))
                    .show_ui(ui, |ui| {
                        for (codec, id) in [
                            (render::VideoCodec::Automatic, "export.codec.auto"),
                            (render::VideoCodec::H264, "export.codec.h264"),
                            (render::VideoCodec::Vp9, "export.codec.vp9"),
                        ] {
                            let r = ui.add_enabled(
                                codec.fits(options.format.container()),
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
                    })
                    .response
            });
            result
                .widgets
                .push(("export.codec", combo.labelled_by(row_label.id)));
            ui.end_row();
            let row_label = ui.label(tr(Text::ExportFormat));
            let shown = if section == Section::Video {
                options.format
            } else {
                last_format(ui, false)
            };
            let combo = gated(ui, locale, video_off, |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.format"))
                    .width(field_width)
                    .selected_text(tr(shown.label()))
                    .show_ui(ui, |ui| {
                        for (format, id) in [
                            (ExportFormat::Mp4, "export.format.mp4"),
                            (ExportFormat::Mov, "export.format.mov"),
                            (ExportFormat::Webm, "export.format.webm"),
                            (ExportFormat::PngSequence, "export.format.png_sequence"),
                        ] {
                            // PNG frames need no encoder from the media build.
                            if !format.is_sequence()
                                && !media::export_container_available(format.container())
                            {
                                continue;
                            }
                            let r =
                                ui.selectable_label(options.format == format, tr(format.label()));
                            if r.clicked() && options.format != format {
                                select_format(options, format);
                            }
                            result.widgets.push((id, r));
                        }
                    })
                    .response
            });
            result
                .widgets
                .push(("export.format", combo.labelled_by(row_label.id)));
            ui.end_row();
            // The frame rate stays live for GIF, whose rate `check_gif` caps.
            let row_label = ui.label(tr(Text::ExportFrameRate));
            let label = options.output.fps.map_or_else(
                || tr(Text::ExportProjectCanvas).to_owned(),
                |fps| format!("{}", fps.as_f64()),
            );
            let fps_off = (section == Section::Audio).then_some(Text::ExportNotForAudio);
            let combo = gated(ui, locale, fps_off, |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.fps"))
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
                    })
                    .response
            });
            result
                .widgets
                .push(("export.fps", combo.labelled_by(row_label.id)));
            ui.end_row();
            let row_label = ui.label(tr(Text::ExportProfile));
            let label = |profile| match profile {
                media::H264Profile::Automatic => tr(Text::ExportAutomatic),
                media::H264Profile::Main => "Main",
                media::H264Profile::High => "High",
            };
            let combo = gated(ui, locale, options.format.unused_encoder_reason(), |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.profile"))
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
                    })
                    .response
            });
            result
                .widgets
                .push(("export.profile", combo.labelled_by(row_label.id)));
            ui.end_row();
        });
    let custom = ui
        .add_enabled_ui(section != Section::Audio, |ui| {
            egui::CollapsingHeader::new(tr(Text::ExportCustomSize))
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
                })
                .header_response
        })
        .inner;
    let custom = if section == Section::Audio {
        custom.on_disabled_hover_text(tr(Text::ExportNotForAudio))
    } else {
        custom
    };
    result.widgets.push(("export.custom.size", custom));
    ui.add_space(12.0);
    // Audio: with Video ticked it is the video's own track; alone it is an audio file.
    let audio_ticked = match section {
        Section::Video => options.include_audio,
        Section::Audio => true,
        Section::Gif => false,
    };
    let audio_refused = if options.format.is_sequence() {
        Some(Text::ExportNotForSequence)
    } else if section == Section::Audio {
        Some(Text::ExportKeepOne)
    } else {
        None
    };
    let (changed, r) = section_header(ui, locale, audio_ticked, Text::IncludeAudio, audio_refused);
    result.widgets.push(("export.audio.include", r));
    if changed {
        if section == Section::Video {
            options.include_audio = !options.include_audio;
        } else {
            select_section(ui, options, project, Section::Audio);
        }
    }
    ui.add_space(8.0);
    egui::Grid::new("export.audio")
        .num_columns(2)
        .min_col_width(115.0)
        .spacing([16.0, 16.0])
        .show(ui, |ui| {
            let row_label = ui.label(tr(Text::ExportFormat));
            let shown = if options.format.audio_only() {
                options.format
            } else {
                last_format(ui, true)
            };
            let off = (Section::of(options.format) != Section::Audio)
                .then_some(Text::ExportAudioFormatOff);
            let combo = gated(ui, locale, off, |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.audio.format"))
                    .width(field_width)
                    .selected_text(tr(shown.label()))
                    .show_ui(ui, |ui| {
                        // AAC is the reference's entry; Opus is ours. MP3, WAV and FLAC need
                        // encoders the media crate does not have.
                        for (format, id) in [
                            (ExportFormat::M4a, "export.audio.format.m4a"),
                            (ExportFormat::AudioWebm, "export.audio.format.opus"),
                        ] {
                            if !media::audio_export_container_available(format.container()) {
                                continue;
                            }
                            let r =
                                ui.selectable_label(options.format == format, tr(format.label()));
                            if r.clicked() && options.format != format {
                                select_format(options, format);
                            }
                            result.widgets.push((id, r));
                        }
                    })
                    .response
            });
            result
                .widgets
                .push(("export.audio.format", combo.labelled_by(row_label.id)));
            ui.end_row();
        });
    ui.add_space(12.0);
    let gif_missing = !media::export_container_available(media::Container::Gif);
    let gif_refused = if section == Section::Gif {
        Some(Text::ExportKeepOne)
    } else if gif_missing {
        Some(Text::ExportCapability)
    } else {
        None
    };
    let (changed, r) = section_header(
        ui,
        locale,
        section == Section::Gif,
        Text::ExportGif,
        gif_refused,
    );
    result.widgets.push(("export.gif.include", r));
    if changed {
        select_section(ui, options, project, Section::Gif);
    }
    ui.add_space(8.0);
    egui::Grid::new("export.gif")
        .num_columns(2)
        .min_col_width(115.0)
        .spacing([16.0, 16.0])
        .show(ui, |ui| {
            let row_label = ui.label(tr(Text::ExportResolution));
            let in_gif = Section::of(options.format) == Section::Gif;
            let label = options.output.resolution.map_or_else(
                || {
                    if in_gif {
                        tr(Text::ExportProjectCanvas).to_owned()
                    } else {
                        GIF_RESOLUTIONS[0].1.to_owned()
                    }
                },
                |size| {
                    GIF_RESOLUTIONS
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
            let combo = gated(ui, locale, (!in_gif).then_some(Text::ExportTickGif), |ui| {
                egui::ComboBox::from_id_salt(egui::IdSalt::new("export.gif.resolution"))
                    .width(field_width)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (h, label, id) in GIF_RESOLUTIONS {
                            let size = resolution_size(project, h);
                            let r = ui.selectable_value(
                                &mut options.output.resolution,
                                Some(size),
                                label,
                            );
                            result.widgets.push((id, r));
                        }
                    })
                    .response
            });
            result
                .widgets
                .push(("export.gif.resolution", combo.labelled_by(row_label.id)));
            ui.end_row();
        });
    ui.add_space(12.0);
    egui::Grid::new("export.scope")
        .num_columns(2)
        .min_col_width(115.0)
        .spacing([16.0, if spacious { 26.0 } else { 16.0 }])
        .show(ui, |ui| {
            scope_rows(ui, locale, options, project, selection, field_width, result);
        });
    if !valid(options, project) {
        ui.colored_label(ui.visuals().error_fg_color, tr(Text::ExportInvalidRange));
    }
}

/// Scene and range: ours, with no counterpart in the reference.
fn scope_rows(
    ui: &mut egui::Ui,
    locale: Locale,
    options: &mut ExportOptions,
    project: &project::Project,
    selection: Option<time::TimeRange>,
    field_width: f32,
    result: &mut DialogUi,
) {
    let tr = |key| i18n::text(locale, key);
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
                // One PNG sequence directory cannot hold several scenes.
                let refused = (value == SceneChoice::All && options.format.is_sequence())
                    .then_some(Text::ExportOneSceneOnly);
                let r = gated(ui, locale, refused, |ui| {
                    ui.selectable_value(&mut options.output.scene, value, label)
                });
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
            let enabled = selection.is_some() && options.output.scene == SceneChoice::Active;
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
}

/// Adds a row control, disabled when the format leaves the row unused. Hovering the
/// disabled control names the reason.
fn gated(
    ui: &mut egui::Ui,
    locale: Locale,
    unused: Option<Text>,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    let response = ui.add_enabled_ui(unused.is_none(), add).inner;
    match unused {
        Some(reason) => response.on_disabled_hover_text(i18n::text(locale, reason)),
        None => response,
    }
}

pub(super) fn valid(options: &ExportOptions, project: &project::Project) -> bool {
    let mut settings = options.settings(project);
    options.output.ranges(project).is_ok()
        && options.output.apply(&mut settings).is_ok()
        && options.output.check_gif(project, &settings).is_ok()
        && options.output.check_sequence(project, &settings).is_ok()
}

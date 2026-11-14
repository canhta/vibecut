//! The 480 px Settings modal: Draft, Edit, Performance and General. A row exists
//! only for a setting the app stores; the cache limit and hardware choice
//! apply after the next start, and say so in the row.
use super::{Action, State};
use crate::{
    editor::{Context, context::CachePhase, egui},
    i18n::{self, Locale, Text},
    preferences::{
        HardwareAcceleration, ImageDurationUnit, LoudnessPreset, PROJECT_FPS, TimeCode,
    },
};

/// Cache budgets offered in MiB beside Automatic; both bounds are included.
const CACHE_LIMITS_MIB: [u64; 7] = [256, 512, 768, 1024, 2048, 4096, 8192];
const FPS_COMBO: &str = "settings.draft.fps";
const HARDWARE_COMBO: &str = "settings.performance.hardware";
const CACHE_COMBO: &str = "settings.performance.cache";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Tab {
    #[default]
    Draft,
    Edit,
    Performance,
    General,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Draft, Self::Edit, Self::Performance, Self::General];

    fn id(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Edit => "edit",
            Self::Performance => "performance",
            Self::General => "general",
        }
    }

    fn label(self) -> Text {
        match self {
            Self::Draft => Text::SettingsTabDraft,
            Self::Edit => Text::SettingsTabEdit,
            Self::Performance => Text::SettingsTabPerformance,
            Self::General => Text::SettingsTabGeneral,
        }
    }
}

/// Keybinding profile transfers; the window opens the native picker for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Shortcuts {
    Export,
    Import { replace: bool },
}

/// The last shortcut transfer's outcome, shown under the General tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ShortcutNotice {
    Imported {
        unknown: usize,
    },
    Exported,
    /// The file was oversized, malformed or from a newer schema; nothing changed.
    Refused,
}

fn mib(bytes: u64) -> u64 {
    bytes >> 20
}

fn hardware_label(choice: HardwareAcceleration) -> Text {
    match choice {
        HardwareAcceleration::Auto => Text::SettingsHardwareAuto,
        HardwareAcceleration::On => Text::SettingsHardwareOn,
        HardwareAcceleration::Off => Text::SettingsHardwareOff,
    }
}

/// Edit tab geometry, logical px from CapCut frames 110800 and 110804: the caption
/// column is 164 wide (x 704 to 868), rows sit 46 apart, a number field with its stepper
/// is 77.5 wide and a select 268.
const ROW_PITCH: f32 = 46.0;
const CAPTION_WIDTH: f32 = 164.0;
const NUMBER_WIDTH: f32 = 61.5;
/// A select is 268 wide, and the unit select beside a number field 59.5.
const SELECT_WIDTH: f32 = 268.0;
const UNIT_SELECT_WIDTH: f32 = 59.5;

/// One Edit row: the caption in its column, then the controls.
fn edit_row(ui: &mut egui::Ui, caption: &str, controls: impl FnOnce(&mut egui::Ui)) {
    edit_row_with(ui, caption, |ui, _| controls(ui));
}

/// An Edit row whose controls also get the caption's response, to label a select by.
fn edit_row_with(
    ui: &mut egui::Ui,
    caption: &str,
    controls: impl FnOnce(&mut egui::Ui, &egui::Response),
) {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_PITCH);
        // The reference left-aligns the caption in its 164 px column (frame 110800).
        let label = ui
            .allocate_ui_with_layout(
                egui::vec2(CAPTION_WIDTH, ROW_PITCH),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| ui.label(caption),
            )
            .inner;
        controls(ui, &label);
    });
}

/// A number field with the kit stepper. Typing commits on Enter; the stepper moves by
/// `step`. `encode` turns the shown value into the stored JSON.
fn number_field(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    (id, caption): (&str, &str),
    shown: (f64, usize),
    limits: (f64, f64, f64),
    (key, encode): (&'static str, impl Fn(f64) -> serde_json::Value),
) {
    let (value, decimals) = shown;
    let (min, max, step) = limits;
    let mut next = value;
    let field = ui.add_sized(
        [NUMBER_WIDTH, crate::kit::size::FIELD],
        egui::DragValue::new(&mut next)
            .range(min..=max)
            .speed(0.0)
            .fixed_decimals(decimals),
    );
    actions.push(Action::Remember(id.into(), field));
    let (up, down) = crate::kit::stepper(ui, id, caption);
    actions.push(Action::Remember(format!("{id}.increase"), up.clone()));
    actions.push(Action::Remember(format!("{id}.decrease"), down.clone()));
    if up.clicked() {
        next = (value + step).min(max);
    }
    if down.clicked() {
        next = (value - step).max(min);
    }
    if (next - value).abs() > f64::EPSILON {
        actions.push(Action::Set(key, encode(next)));
    }
}

/// A number row: the field with its stepper, then the unit.
fn number_row(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    (id, caption, unit): (&str, &str, &str),
    shown: (f64, usize),
    limits: (f64, f64, f64),
    set: (&'static str, impl Fn(f64) -> serde_json::Value),
) {
    edit_row(ui, caption, |ui| {
        number_field(ui, actions, (id, caption), shown, limits, set);
        ui.label(unit);
    });
}

/// A select of `options` (value, id suffix, label); choosing one sets `key`.
fn combo<T: Copy + PartialEq + serde::Serialize>(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    (id, caption): (&str, egui::Id),
    width: f32,
    (current, current_label): (T, &str),
    options: &[(T, &str, &str)],
    key: &'static str,
) {
    let combo = egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(current_label)
        .show_ui(ui, |ui| {
            for &(choice, suffix, label) in options {
                let response = ui.selectable_label(choice == current, label);
                actions.push(Action::Remember(
                    format!("{id}.{suffix}"),
                    response.clone(),
                ));
                if response.clicked() && choice != current {
                    actions.push(Action::Set(
                        key,
                        serde_json::to_value(choice).unwrap_or_default(),
                    ));
                }
            }
        });
    actions.push(Action::Remember(
        id.into(),
        combo.response.labelled_by(caption),
    ));
}

fn loudness_text(preset: LoudnessPreset) -> Text {
    match preset {
        LoudnessPreset::Standard => Text::SettingsLoudnessStandard,
        LoudnessPreset::Youtube => Text::SettingsLoudnessYoutube,
        LoudnessPreset::Tiktok => Text::SettingsLoudnessTiktok,
    }
}

fn loudness_id(preset: LoudnessPreset) -> &'static str {
    match preset {
        LoudnessPreset::Standard => "standard",
        LoudnessPreset::Youtube => "youtube",
        LoudnessPreset::Tiktok => "tiktok",
    }
}

fn time_code_text(code: TimeCode) -> Text {
    match code {
        TimeCode::SecondsPlusFrame => Text::SettingsTimeCodeSecondsPlusFrame,
        TimeCode::AllColons => Text::SettingsTimeCodeAllColons,
    }
}

fn time_code_id(code: TimeCode) -> &'static str {
    match code {
        TimeCode::SecondsPlusFrame => "seconds_plus_frame",
        TimeCode::AllColons => "all_colons",
    }
}

fn image_unit_text(unit: ImageDurationUnit) -> Text {
    match unit {
        ImageDurationUnit::Seconds => Text::SettingsSecondsUnit,
        ImageDurationUnit::Frames => Text::SettingsFramesUnit,
    }
}

fn image_unit_id(unit: ImageDurationUnit) -> &'static str {
    match unit {
        ImageDurationUnit::Seconds => "seconds",
        ImageDurationUnit::Frames => "frames",
    }
}

fn toggle(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    id: &str,
    label: String,
    value: bool,
    key: &'static str,
) {
    let mut next = value;
    let response = ui.checkbox(&mut next, label);
    actions.push(Action::Remember(id.into(), response.clone()));
    if response.changed() {
        actions.push(Action::Set(key, serde_json::json!(next)));
    }
}

pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ctx: &egui::Context,
    actions: &mut Vec<Action>,
) {
    let locale: Locale = cx.locale;
    let tr = |text: Text| text.text(locale);
    let prefs = cx.preferences;
    let modal = egui::Modal::new(egui::Id::unique("settings"))
        .frame(crate::kit::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(480.0);
            ui.spacing_mut().interact_size.y = crate::kit::size::FIELD;
            crate::kit::dialog_title(ui, tr(Text::Settings));
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    let response = ui.selectable_label(state.tab == tab, tr(tab.label()));
                    actions.push(Action::Remember(
                        format!("settings.tab.{}", tab.id()),
                        response.clone(),
                    ));
                    if response.clicked() {
                        state.tab = tab;
                    }
                }
            });
            ui.separator();
            match state.tab {
                Tab::Draft => {
                    ui.label(tr(Text::SettingsDefaultFpsHint));
                    ui.horizontal(|ui| {
                        let caption = ui.label(tr(Text::SettingsDefaultFps));
                        let current = prefs.default_fps;
                        let combo = egui::ComboBox::from_id_salt(FPS_COMBO)
                            .selected_text(i18n::format(
                                locale,
                                Text::SettingsFps,
                                &[("fps", &current.to_string())],
                            ))
                            .show_ui(ui, |ui| {
                                for fps in PROJECT_FPS {
                                    let label = i18n::format(
                                        locale,
                                        Text::SettingsFps,
                                        &[("fps", &fps.to_string())],
                                    );
                                    let response = ui.selectable_label(fps == current, label);
                                    actions.push(Action::Remember(
                                        format!("settings.draft.fps.{fps}"),
                                        response.clone(),
                                    ));
                                    if response.clicked() && fps != current {
                                        actions.push(Action::Set(
                                            "default_fps",
                                            serde_json::json!(fps),
                                        ));
                                    }
                                }
                            });
                        actions.push(Action::Remember(
                            "settings.draft.fps".into(),
                            combo.response.labelled_by(caption.id),
                        ));
                    });
                }
                Tab::Edit => {
                    let steps = (
                        f64::from(*crate::preferences::STEP_RANGE.start()),
                        f64::from(*crate::preferences::STEP_RANGE.end()),
                        1.0,
                    );
                    number_row(
                        ui,
                        actions,
                        (
                            "settings.edit.frame_step",
                            tr(Text::SettingsStepMany),
                            tr(Text::SettingsFramesUnit),
                        ),
                        (f64::from(prefs.frame_step), 0),
                        steps,
                        ("frame_step", |v: f64| serde_json::json!(v.round() as u32)),
                    );
                    number_row(
                        ui,
                        actions,
                        (
                            "settings.edit.value_step",
                            tr(Text::SettingsAdjustMany),
                            tr(Text::SettingsUnitsUnit),
                        ),
                        (f64::from(prefs.value_step), 0),
                        steps,
                        ("value_step", |v: f64| serde_json::json!(v.round() as u32)),
                    );
                    let image_ms = crate::preferences::IMAGE_DURATION_RANGE_MS;
                    let fps = f64::from(prefs.default_fps);
                    let unit = prefs.image_duration_unit;
                    let (shown, limits) = match unit {
                        ImageDurationUnit::Seconds => (
                            (f64::from(prefs.image_duration_ms) / 1000.0, 1),
                            (
                                f64::from(*image_ms.start()) / 1000.0,
                                f64::from(*image_ms.end()) / 1000.0,
                                0.1,
                            ),
                        ),
                        ImageDurationUnit::Frames => (
                            ((f64::from(prefs.image_duration_ms) * fps / 1000.0).round(), 0),
                            (
                                (f64::from(*image_ms.start()) * fps / 1000.0).ceil(),
                                (f64::from(*image_ms.end()) * fps / 1000.0).floor(),
                                1.0,
                            ),
                        ),
                    };
                    let caption = tr(Text::SettingsImageDuration);
                    edit_row_with(ui, caption, |ui, label| {
                        number_field(
                            ui,
                            actions,
                            ("settings.edit.image_duration", caption),
                            shown,
                            limits,
                            ("image_duration_ms", |v: f64| {
                                let ms = match unit {
                                    ImageDurationUnit::Seconds => v * 1000.0,
                                    ImageDurationUnit::Frames => v * 1000.0 / fps,
                                };
                                serde_json::json!((ms.round() as u32)
                                    .clamp(*image_ms.start(), *image_ms.end()))
                            }),
                        );
                        let options = ImageDurationUnit::ALL
                            .map(|u| (u, image_unit_id(u), tr(image_unit_text(u))));
                        combo(
                            ui,
                            actions,
                            ("settings.edit.image_duration_unit", label.id),
                            UNIT_SELECT_WIDTH,
                            (unit, tr(image_unit_text(unit))),
                            &options,
                            "image_duration_unit",
                        );
                    });
                    let preset = prefs.loudness_preset;
                    let options =
                        LoudnessPreset::ALL.map(|p| (p, loudness_id(p), tr(loudness_text(p))));
                    edit_row_with(ui, tr(Text::SettingsTargetLoudness), |ui, label| {
                        combo(
                            ui,
                            actions,
                            ("settings.edit.loudness_preset", label.id),
                            SELECT_WIDTH,
                            (preset, tr(loudness_text(preset))),
                            &options,
                            "loudness_preset",
                        );
                    });
                    let code = prefs.time_code;
                    let options = TimeCode::ALL.map(|c| (c, time_code_id(c), tr(time_code_text(c))));
                    edit_row_with(ui, tr(Text::SettingsTimeCode), |ui, label| {
                        combo(
                            ui,
                            actions,
                            ("settings.edit.time_code", label.id),
                            SELECT_WIDTH,
                            (code, tr(time_code_text(code))),
                            &options,
                            "time_code",
                        );
                    });
                    ui.separator();
                    edit_row(ui, tr(Text::SettingsAudioScrub), |ui| {
                        ui.vertical(|ui| {
                            let mut on = prefs.audio_scrub;
                            let response = crate::kit::toggle(
                                ui,
                                "settings.edit.audio_scrub",
                                &mut on,
                                tr(Text::SettingsAudioScrub),
                            );
                            actions.push(Action::Remember(
                                "settings.edit.audio_scrub".into(),
                                response,
                            ));
                            ui.label(egui::RichText::new(tr(Text::SettingsAudioScrubHint)).small());
                            if on != prefs.audio_scrub {
                                actions.push(Action::Set("audio_scrub", serde_json::json!(on)));
                            }
                        });
                    });
                    edit_row(ui, tr(Text::SettingsExportSound), |ui| {
                        let mut on = prefs.export_sound;
                        let response = crate::kit::checkbox(
                            ui,
                            "settings.edit.export_sound",
                            &mut on,
                            tr(Text::SettingsExportSoundHint),
                        );
                        actions.push(Action::Remember(
                            "settings.edit.export_sound".into(),
                            response,
                        ));
                        if on != prefs.export_sound {
                            actions.push(Action::Set("export_sound", serde_json::json!(on)));
                        }
                    });
                    ui.separator();
                    toggle(
                        ui,
                        actions,
                        "settings.edit.snapping",
                        tr(Text::SettingsSnapping).into(),
                        prefs.snapping,
                        "snapping",
                    );
                    toggle(
                        ui,
                        actions,
                        "settings.edit.ripple",
                        tr(Text::SettingsRipple).into(),
                        prefs.ripple,
                        "ripple",
                    );
                    toggle(
                        ui,
                        actions,
                        "settings.edit.preview_axis",
                        tr(Text::SettingsPreviewAxis).into(),
                        prefs.preview_axis,
                        "preview_axis",
                    );
                }
                Tab::Performance => {
                    ui.label(tr(Text::SettingsHardwareHint));
                    ui.horizontal(|ui| {
                        let caption = ui.label(tr(Text::SettingsHardware));
                        let current = prefs.hardware_acceleration;
                        let combo = egui::ComboBox::from_id_salt(HARDWARE_COMBO)
                            .selected_text(tr(hardware_label(current)))
                            .show_ui(ui, |ui| {
                                for choice in [
                                    HardwareAcceleration::Auto,
                                    HardwareAcceleration::On,
                                    HardwareAcceleration::Off,
                                ] {
                                    let response = ui.selectable_label(
                                        choice == current,
                                        tr(hardware_label(choice)),
                                    );
                                    actions.push(Action::Remember(
                                        format!("settings.performance.hardware.{choice:?}"),
                                        response.clone(),
                                    ));
                                    if response.clicked() && choice != current {
                                        actions.push(Action::Set(
                                            "hardware_acceleration",
                                            serde_json::to_value(choice).unwrap_or_default(),
                                        ));
                                    }
                                }
                            });
                        actions.push(Action::Remember(
                            "settings.performance.hardware".into(),
                            combo.response.labelled_by(caption.id),
                        ));
                    });
                    ui.label(tr(Text::SettingsCacheHint));
                    ui.horizontal(|ui| {
                        let caption = ui.label(tr(Text::SettingsCacheLimit));
                        let current = prefs.cache_limit_bytes;
                        let combo = egui::ComboBox::from_id_salt(CACHE_COMBO)
                            .selected_text(match current {
                                Some(bytes) => i18n::format(
                                    locale,
                                    Text::SettingsMib,
                                    &[("mib", &mib(bytes).to_string())],
                                ),
                                None => tr(Text::SettingsHardwareAuto).to_string(),
                            })
                            .show_ui(ui, |ui| {
                                let auto = ui.selectable_label(
                                    current.is_none(),
                                    tr(Text::SettingsHardwareAuto),
                                );
                                actions.push(Action::Remember(
                                    "settings.performance.cache.auto".into(),
                                    auto.clone(),
                                ));
                                if auto.clicked() && current.is_some() {
                                    actions.push(Action::Set(
                                        "cache_limit_bytes",
                                        serde_json::Value::Null,
                                    ));
                                }
                                for limit in CACHE_LIMITS_MIB {
                                    let bytes = limit << 20;
                                    let label = i18n::format(
                                        locale,
                                        Text::SettingsMib,
                                        &[("mib", &limit.to_string())],
                                    );
                                    let response =
                                        ui.selectable_label(Some(bytes) == current, label);
                                    actions.push(Action::Remember(
                                        format!("settings.performance.cache.{limit}"),
                                        response.clone(),
                                    ));
                                    if response.clicked() && Some(bytes) != current {
                                        actions.push(Action::Set(
                                            "cache_limit_bytes",
                                            serde_json::json!(bytes),
                                        ));
                                    }
                                }
                            });
                        actions.push(Action::Remember(
                            "settings.performance.cache".into(),
                            combo.response.labelled_by(caption.id),
                        ));
                    });
                    let cache = &cx.cache;
                    let clearing = cache.phase == CachePhase::Clearing;
                    ui.horizontal(|ui| {
                        let response =
                            ui.add_enabled(!clearing, egui::Button::new(tr(Text::CacheClear)));
                        actions.push(Action::Remember(
                            "settings.performance.cache.clear".into(),
                            response.clone(),
                        ));
                        if response.clicked() {
                            actions.push(Action::ClearCache);
                        }
                        if let Some(report) = &cache.report {
                            ui.label(i18n::format(
                                locale,
                                Text::SettingsCacheFreed,
                                &[
                                    ("freed", &mib(report.removed_bytes).to_string()),
                                    (
                                        "after",
                                        &mib(report
                                            .disk_bytes
                                            .saturating_sub(report.removed_bytes))
                                        .to_string(),
                                    ),
                                    ("kept", &report.in_use.to_string()),
                                ],
                            ));
                        }
                    });
                    let status = match cache.phase {
                        CachePhase::Clearing => Some(Text::CacheClearing),
                        CachePhase::Inspecting => Some(Text::CacheInspecting),
                        CachePhase::Cancelling => Some(Text::CacheCancelling),
                        _ => None,
                    };
                    if let Some(status) = status {
                        ui.label(tr(status));
                    }
                    // A clear that raced an active job reports CacheBusy here, typed.
                    if let Some(problem) = cache.problem() {
                        ui.label(
                            egui::RichText::new(tr(problem)).color(crate::kit::warning(cx.dark)),
                        );
                    }
                }
                Tab::General => {
                    let other = match prefs.locale {
                        Locale::En => Locale::Vi,
                        Locale::Vi => Locale::En,
                    };
                    ui.horizontal(|ui| {
                        ui.label(tr(Text::SettingsLanguage));
                        let response = ui.button(tr(Text::Language));
                        actions.push(Action::Remember(
                            "settings.general.language".into(),
                            response.clone(),
                        ));
                        if response.clicked() {
                            actions.push(Action::Set("locale", serde_json::json!(other)));
                        }
                    });
                    toggle(
                        ui,
                        actions,
                        "settings.general.dark",
                        tr(Text::SettingsDark).into(),
                        prefs.dark,
                        "dark",
                    );
                    ui.separator();
                    ui.strong(tr(Text::SettingsShortcuts));
                    for (id, label, request) in [
                        (
                            "settings.general.shortcuts.export",
                            Text::SettingsExportShortcuts,
                            Shortcuts::Export,
                        ),
                        (
                            "settings.general.shortcuts.merge",
                            Text::SettingsImportMerge,
                            Shortcuts::Import { replace: false },
                        ),
                        (
                            "settings.general.shortcuts.replace",
                            Text::SettingsImportReplace,
                            Shortcuts::Import { replace: true },
                        ),
                    ] {
                        let response = ui.button(tr(label));
                        actions.push(Action::Remember(id.into(), response.clone()));
                        if response.clicked() {
                            actions.push(Action::Shortcuts(request));
                        }
                    }
                    match state.shortcut_notice {
                        Some(ShortcutNotice::Imported { unknown }) => {
                            ui.label(i18n::format(
                                locale,
                                Text::ShortcutsImported,
                                &[("count", &unknown.to_string())],
                            ));
                        }
                        Some(ShortcutNotice::Exported) => {
                            ui.label(tr(Text::ShortcutsExported));
                        }
                        Some(ShortcutNotice::Refused) => {
                            ui.label(
                                egui::RichText::new(tr(Text::ShortcutsRefused))
                                    .color(crate::kit::warning(cx.dark)),
                            );
                        }
                        None => {}
                    }
                    ui.separator();
                    let response = ui.button(tr(Text::SettingsModels));
                    actions.push(Action::Remember(
                        "settings.general.models".into(),
                        response.clone(),
                    ));
                    if response.clicked() {
                        state.models_open = true;
                    }
                }
            }
            ui.separator();
            let response =
                crate::kit::dialog_button(ui, "settings.close", tr(Text::ModelClose), false);
            actions.push(Action::Remember("settings.close".into(), response.clone()));
            if response.clicked() {
                state.open = false;
            }
        });
    if modal.should_close() {
        state.open = false;
    }
}

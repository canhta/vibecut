//! The 480 px Settings modal: Draft, Edit, Performance and General. A row exists
//! only for a setting the app stores; the cache limit and hardware choice
//! apply after the next start, and say so in the row.
use super::{Action, State};
use crate::{
    editor::{Context, egui},
    i18n::{self, Locale, Text},
    preferences::{DEFAULT_CACHE_LIMIT, HardwareAcceleration, PROJECT_FPS},
};

/// Cache budgets offered in MiB; the default and both bounds are included.
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
    Imported { unknown: usize },
    Exported,
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
                        ui.label(tr(Text::SettingsDefaultFps));
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
                            combo.response.clone(),
                        ));
                    });
                }
                Tab::Edit => {
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
                        ui.label(tr(Text::SettingsHardware));
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
                            combo.response.clone(),
                        ));
                    });
                    ui.label(tr(Text::SettingsCacheHint));
                    ui.horizontal(|ui| {
                        ui.label(tr(Text::SettingsCacheLimit));
                        let current = prefs.cache_limit_bytes;
                        let combo = egui::ComboBox::from_id_salt(CACHE_COMBO)
                            .selected_text(i18n::format(
                                locale,
                                Text::SettingsMib,
                                &[("mib", &mib(current).to_string())],
                            ))
                            .show_ui(ui, |ui| {
                                for limit in CACHE_LIMITS_MIB {
                                    let bytes = limit << 20;
                                    let label = i18n::format(
                                        locale,
                                        Text::SettingsMib,
                                        &[("mib", &limit.to_string())],
                                    );
                                    let response = ui.selectable_label(bytes == current, label);
                                    actions.push(Action::Remember(
                                        format!("settings.performance.cache.{limit}"),
                                        response.clone(),
                                    ));
                                    if response.clicked() && bytes != current {
                                        actions.push(Action::Set(
                                            "cache_limit_bytes",
                                            serde_json::json!(bytes),
                                        ));
                                    }
                                }
                            });
                        actions.push(Action::Remember(
                            "settings.performance.cache".into(),
                            combo.response.clone(),
                        ));
                    });
                    // The default must be one of the offered choices.
                    debug_assert!(CACHE_LIMITS_MIB.contains(&mib(DEFAULT_CACHE_LIMIT)));
                    ui.horizontal(|ui| {
                        let response = ui.button(tr(Text::CacheClear));
                        actions.push(Action::Remember(
                            "settings.performance.cache.clear".into(),
                            response.clone(),
                        ));
                        if response.clicked() {
                            actions.push(Action::ClearCache);
                        }
                        if let Some(report) = &cx.cache.report {
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
                crate::kit::dialog_button(ui, "settings.close", &tr(Text::ModelClose), false);
            actions.push(Action::Remember("settings.close".into(), response.clone()));
            if response.clicked() {
                state.open = false;
            }
        });
    if modal.should_close() {
        state.open = false;
    }
}

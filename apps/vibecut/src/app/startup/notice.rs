//! The startup notices Home shows in one quiet toast, and the release check behind the first.
use super::{crash, update};
use crate::{
    egui,
    i18n::{self, Text},
    kit::{self, Icon, size, type_size},
    preferences::Preferences,
};
use std::{
    path::PathBuf,
    sync::Arc,
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

/// A newer release to open and the crash folder to reveal, each until dismissed.
pub(in crate::app) struct Notices {
    check: Option<JoinHandle<Option<update::Release>>>,
    release: Option<update::Release>,
    /// The crash folder, while the last run's report is offered.
    crash: Option<PathBuf>,
    folder: PathBuf,
    /// The folder Open folder asked for, until the native shell takes it.
    reveal: Option<PathBuf>,
}

impl Notices {
    /// Read the crash folder once: an unread report from the last run is offered.
    pub(in crate::app) fn new(folder: PathBuf) -> Self {
        Self {
            check: None,
            release: None,
            crash: crash::take_unread(&folder).then(|| folder.clone()),
            folder,
            reveal: None,
        }
    }
    /// Start the release check on a worker when it is due, and record the start in the
    /// preferences so the next one waits a day. The worker wakes the loop when it ends.
    pub(in crate::app) fn check(
        &mut self,
        preferences: &mut Preferences,
        transport: Arc<dyn transport::HttpTransport>,
        now: SystemTime,
    ) {
        let now = now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        if self.check.is_some()
            || !update::due(preferences.update_check, preferences.update_checked_at, now)
        {
            return;
        }
        preferences.update_checked_at = Some(now);
        self.check = Some(wake::spawn(move || update::newer(transport.as_ref())));
    }
    pub(in crate::app) fn checking(&self) -> bool {
        self.check.is_some()
    }
    /// Take a finished check's answer; a failed worker is as silent as no newer release.
    pub(in crate::app) fn tick(&mut self) {
        if self.check.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(check) = self.check.take()
        {
            self.release = check.join().ok().flatten();
        }
    }
    pub(in crate::app) fn take_reveal(&mut self) -> Option<PathBuf> {
        self.reveal.take()
    }
    /// The toast at Home's bottom right: one row per notice, its text then its icon actions.
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context, look: kit::Look) {
        if self.release.is_none() && self.crash.is_none() {
            return;
        }
        let tr = |text: Text| i18n::text(look.locale, text);
        let (mut open, mut dismiss_release, mut reveal, mut dismiss_crash) =
            (false, false, false, false);
        kit::toast(ctx, "startup.toast", |ui| {
            if let Some(release) = &self.release {
                let line = i18n::format(
                    look.locale,
                    Text::UpdateAvailable,
                    &[("version", release.tag())],
                );
                row(ui, &line, look.dark, |ui| {
                    dismiss_release = kit::icon_button(
                        ui,
                        "update.dismiss",
                        Icon::X,
                        tr(Text::NoticeDismiss),
                        false,
                    )
                    .clicked();
                    open = kit::icon_button(
                        ui,
                        "update.open",
                        Icon::LinkSimple,
                        tr(Text::UpdateOpen),
                        false,
                    )
                    .clicked();
                });
            }
            if self.crash.is_some() {
                row(ui, tr(Text::CrashNotice), look.dark, |ui| {
                    dismiss_crash = kit::icon_button(
                        ui,
                        "crash.dismiss",
                        Icon::X,
                        tr(Text::NoticeDismiss),
                        false,
                    )
                    .clicked();
                    reveal = kit::icon_button(
                        ui,
                        "crash.open_folder",
                        Icon::FolderOpen,
                        tr(Text::CrashOpenFolder),
                        false,
                    )
                    .clicked();
                });
            }
        });
        if open && let Some(release) = self.release.take() {
            ctx.open_url(egui::OpenUrl::new_tab(release.page()));
        }
        if dismiss_release {
            self.release = None;
        }
        if reveal {
            self.crash = None;
            self.reveal = Some(self.folder.clone());
        }
        if dismiss_crash {
            self.crash = None;
        }
    }
}

/// One notice row: the icon actions at the right edge (drawn right to left, so dismiss comes
/// first), then the line wrapping in the room left of them.
fn row(ui: &mut egui::Ui, text: &str, dark: bool, actions: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, size::CONTROL),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            actions(ui);
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .size(type_size::BODY)
                                .color(kit::text(dark)),
                        )
                        .wrap(),
                    );
                },
            );
        },
    );
}

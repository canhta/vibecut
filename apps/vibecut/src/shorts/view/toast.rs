//! A bottom-right notice: the last failure and the export jobs.
use super::{State, widgets};
use crate::{
    egui::{self, Align2, vec2},
    i18n::Text,
    kit::{self, Icon},
    shorts::state::{failure_text, terminal},
};
use engine::jobs::JobState;

pub(super) fn show(state: &mut State, ui: &mut egui::Ui) {
    let dark = state.preferences.dark;
    let jobs = state.batch.as_ref().map_or(0, |b| b.len());
    if state.error.is_none() && !state.cancelled && jobs == 0 {
        return;
    }
    let mut dismiss = false;
    let mut retry = None;
    let mut cancel = None;
    egui::Area::new(egui::Id::unique("shorts.toast"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(kit::modal(dark))
                .corner_radius(12)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    if state.error.is_some() || state.cancelled {
                        let (text, color) = if state.error.is_some() {
                            (state.tr(state.error_text), kit::error_text(dark))
                        } else {
                            (state.tr(Text::ShortsCancelled), kit::text(dark))
                        };
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [260.0, 24.0],
                                egui::Label::new(egui::RichText::new(text).color(color)).wrap(),
                            );
                            let rect = ui.allocate_space(vec2(24.0, 24.0)).1;
                            if widgets::icon_tap(
                                ui,
                                "shorts.error.dismiss",
                                rect,
                                Icon::X,
                                state.tr(Text::ShortsDismiss),
                                true,
                                false,
                            )
                            .clicked()
                            {
                                dismiss = true;
                            }
                        });
                    }
                    let Some(batch) = &state.batch else { return };
                    for index in 0..batch.len() {
                        let Some(snapshot) = batch.snapshot(&state.runner, index) else {
                            continue;
                        };
                        ui.horizontal(|ui| {
                            if let Some(paths) = batch.paths(index) {
                                let name = paths
                                    .video
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy();
                                ui.add_sized(
                                    [150.0, 24.0],
                                    egui::Label::new(name.as_ref()).truncate(),
                                )
                                .on_hover_text(name.as_ref());
                            }
                            let key = match snapshot.state {
                                JobState::Completed => Text::ShortsDone,
                                JobState::Failed => Text::ShortsJobFailed,
                                JobState::Cancelled => Text::ShortsJobCancelled,
                                _ => Text::Processing,
                            };
                            ui.label(state.tr(key));
                            let rect = ui.allocate_space(vec2(24.0, 24.0)).1;
                            if matches!(snapshot.state, JobState::Failed | JobState::Cancelled)
                                && widgets::icon_tap(
                                    ui,
                                    &format!("shorts.export.{index}.retry"),
                                    rect,
                                    Icon::ArrowClockwise,
                                    state.tr(Text::Retry),
                                    true,
                                    false,
                                )
                                .clicked()
                            {
                                retry = Some(index);
                            }
                            if !terminal(snapshot.state)
                                && widgets::icon_tap(
                                    ui,
                                    &format!("shorts.export.{index}.cancel"),
                                    rect,
                                    Icon::X,
                                    state.tr(Text::Cancel),
                                    true,
                                    false,
                                )
                                .clicked()
                            {
                                cancel = Some(index);
                            }
                        });
                        if let Some(error) = &snapshot.error {
                            ui.label(state.tr(failure_text(error)));
                        }
                        if !terminal(snapshot.state) {
                            crate::export::job_progress(ui, &snapshot, state.preferences.locale);
                        }
                    }
                });
        });
    if dismiss {
        state.error = None;
        state.cancelled = false;
    }
    if let Some(index) = cancel
        && let Some(batch) = &state.batch
        && let Err(e) = batch.cancel(index)
    {
        state.fail(e.into(), Text::ShortsError);
    }
    if let Some(index) = retry
        && let Err(e) = state.retry_export(index)
    {
        state.fail(e, Text::ShortsError);
    }
}

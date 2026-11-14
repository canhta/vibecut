//! Reviews scene-change candidates of one video clip before the reviewed cuts are split.
//!
//! Finding the cuts is the engine's `media.detect_scenes` job, admitted through control and
//! polled with `job_status`; Cancel sends `job_cancel`. Apply hands the reviewed indexes to
//! `scene_detection::apply_reviewed`, which splits the clip in one undo step. The dialog
//! never writes the project itself.
#[cfg(test)]
mod tests;

use crate::{
    editor::{Context, Port, egui},
    i18n::{self, Text},
    kit,
    services::progress,
};
use engine::{
    ByteBudget, Command, Session,
    control::{Caller, ControlResult, JobOutput, MediaAccess, Policy},
    jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock},
    scene_detection::{ChangeKind, DetectionResult, DetectionSettings, apply_reviewed},
};
use media::AssetId;
use project::{ItemId, ItemParams};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use time::{TICKS_PER_SECOND, Tick};

/// The desktop owns this clip's detection; the trusted caller needs no further policy.
struct Allow;

impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

static ALLOW: Allow = Allow;

/// Where the dialog is in its review, one state at a time.
#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Detecting {
        job: JobId,
        progress: f32,
        cancelling: bool,
    },
    Review {
        result: DetectionResult,
        selected: Vec<bool>,
    },
    NoCuts,
    Cancelled,
    Failed(Text),
    /// One frame of "Splitting" is painted before the reviewed cuts are committed.
    Applying {
        result: DetectionResult,
        selected: Vec<bool>,
    },
}

/// The dialog's state across frames. A running detection is cancelled when the dialog closes.
#[derive(Default)]
pub(in crate::editor) struct State {
    open: bool,
    item: Option<ItemId>,
    phase: Phase,
    media: Option<Arc<Mutex<MediaAccess>>>,
    runner: Option<JobRunner>,
    source: Option<DetectSource>,
}

/// Starts the detection job for one asset and returns its id, or `None` when it refuses.
/// Injected by tests to hold a job open deterministically; the desktop uses `media.detect_scenes`.
pub(in crate::editor) type DetectSource =
    Arc<dyn Fn(&mut JobRunner, AssetId) -> Option<JobId> + Send + Sync>;

/// What the dialog asks the frame to do after it painted.
pub(in crate::editor) enum Action {
    Remember(String, egui::Response),
    Find,
    Poll,
    CancelDetect,
    Toggle(usize, bool),
    Apply,
    Commit,
    Close,
}

impl State {
    /// Inject the media cache the detection job runs on.
    pub(in crate::editor) fn set_media(&mut self, media: Arc<Mutex<MediaAccess>>) {
        self.media = Some(media);
    }

    /// Replace the detection job's submission. Scripted jobs share the dialog's runner, so
    /// their gates and the dialog's own polling and cancel are the production path.
    #[cfg(test)]
    pub(in crate::editor) fn set_detect_source(&mut self, source: DetectSource) {
        self.source = Some(source);
    }

    pub(in crate::editor) fn is_open(&self) -> bool {
        self.open
    }

    /// Open for the single selected video clip. Any other selection is refused with the line
    /// the editor shows.
    pub(in crate::editor) fn open(&mut self, session: &Session) -> Result<(), Text> {
        let selected: Vec<ItemId> = session.selection().iter().copied().collect();
        let [item] = selected.as_slice() else {
            return Err(Text::SceneSplitSelectOne);
        };
        if video_source(session, *item).is_none() {
            return Err(Text::SceneSplitSelectOne);
        }
        self.open = true;
        self.item = Some(*item);
        self.phase = Phase::Idle;
        Ok(())
    }
}

/// Paint the dialog and return what it asks for.
pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if !state.open {
        return actions;
    }
    let modal = egui::Modal::new(egui::Id::unique("scene_split.dialog"))
        .frame(kit::dialog_frame(ui.ctx()))
        .show(ui.ctx(), |ui| {
            kit::dialog_title(ui, Text::SceneSplitTitle.text(cx.locale));
            body(&*state, cx, ui, &mut actions);
        });
    if modal.should_close() {
        actions.push(Action::Close);
    }
    match &state.phase {
        Phase::Detecting { .. } => {
            actions.push(Action::Poll);
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        Phase::Applying { .. } => actions.push(Action::Commit),
        _ => {}
    }
    actions
}

fn body(state: &State, cx: &Context<'_>, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let tr = |text: Text| text.text(cx.locale);
    match &state.phase {
        Phase::Idle => {
            ui.label(tr(Text::SceneSplitIdle));
            ui.horizontal(|ui| {
                button(
                    ui,
                    actions,
                    Footer::find(tr(Text::SceneSplitFind), true, true),
                );
                button(ui, actions, Footer::close(tr(Text::SceneSplitClose)));
            });
        }
        Phase::Detecting {
            progress,
            cancelling,
            ..
        } => {
            progress::line(
                ui,
                "scene_split.progress",
                cx.locale,
                tr(Text::SceneSplitDetecting),
                *progress,
            );
            button(
                ui,
                actions,
                Footer {
                    id: "scene_split.cancel",
                    label: tr(Text::Cancel).to_owned(),
                    primary: false,
                    enabled: !*cancelling,
                    on_click: Action::CancelDetect,
                },
            );
        }
        Phase::Review { result, selected } => {
            let found = i18n::format_count(
                cx.locale,
                result.candidates.len() as u64,
                Text::SceneSplitFoundOne,
                Text::SceneSplitFoundOther,
            );
            ui.label(found);
            for (index, candidate) in result.candidates.iter().enumerate() {
                let mut on = selected.get(index).copied().unwrap_or(false);
                let kind = match candidate.kind {
                    ChangeKind::HardCut => tr(Text::SceneSplitHardCut),
                    ChangeKind::Fade => tr(Text::SceneSplitFade),
                };
                let label = format!("{} · {kind}", seconds(candidate.at));
                let id = format!("scene_split.candidate.{index}");
                let response = kit::checkbox(ui, &id, &mut on, &label);
                let changed = response.changed();
                actions.push(Action::Remember(id, response));
                if changed {
                    actions.push(Action::Toggle(index, on));
                }
            }
            let count = selected.iter().filter(|on| **on).count();
            let apply = i18n::format_count(
                cx.locale,
                count as u64,
                Text::SceneSplitApplyOne,
                Text::SceneSplitApplyOther,
            );
            // One panel row of air between the last candidate and the footer.
            ui.add_space(kit::space::LG);
            ui.horizontal(|ui| {
                button(
                    ui,
                    actions,
                    Footer {
                        id: "scene_split.apply",
                        label: apply,
                        primary: true,
                        enabled: count > 0,
                        on_click: Action::Apply,
                    },
                );
                button(ui, actions, Footer::close(tr(Text::SceneSplitClose)));
            });
        }
        Phase::NoCuts => {
            ui.label(tr(Text::SceneSplitNone));
            ui.horizontal(|ui| {
                button(ui, actions, Footer::close(tr(Text::SceneSplitClose)));
            });
        }
        Phase::Cancelled => {
            ui.label(tr(Text::SceneSplitCancelled));
            ui.horizontal(|ui| {
                button(
                    ui,
                    actions,
                    Footer::find(tr(Text::SceneSplitFind), true, true),
                );
                button(ui, actions, Footer::close(tr(Text::SceneSplitClose)));
            });
        }
        Phase::Failed(text) => {
            ui.colored_label(kit::error_text(cx.dark), tr(*text));
            ui.horizontal(|ui| {
                button(
                    ui,
                    actions,
                    Footer::find(tr(Text::SceneSplitFind), true, true),
                );
                button(ui, actions, Footer::close(tr(Text::SceneSplitClose)));
            });
        }
        Phase::Applying { .. } => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr(Text::SceneSplitApplying));
            });
        }
    }
}

/// One footer button: its automation id, its label and the action a click requests.
struct Footer {
    id: &'static str,
    label: String,
    primary: bool,
    enabled: bool,
    on_click: Action,
}

impl Footer {
    fn find(label: &str, primary: bool, enabled: bool) -> Self {
        Self {
            id: "scene_split.find",
            label: label.to_owned(),
            primary,
            enabled,
            on_click: Action::Find,
        }
    }
    fn close(label: &str) -> Self {
        Self {
            id: "scene_split.close",
            label: label.to_owned(),
            primary: false,
            enabled: true,
            on_click: Action::Close,
        }
    }
}

/// A footer button that records its automation id; `enabled` greys it without hiding it.
fn button(ui: &mut egui::Ui, actions: &mut Vec<Action>, footer: Footer) {
    let response = ui
        .add_enabled_ui(footer.enabled, |ui| {
            kit::dialog_button(ui, footer.id, &footer.label, footer.primary)
        })
        .inner;
    if response.clicked() {
        actions.push(footer.on_click);
    }
    actions.push(Action::Remember(footer.id.to_owned(), response));
}

pub(in crate::editor) fn apply(state: &mut State, port: &mut Port<'_>, action: Action) {
    match action {
        Action::Remember(id, response) => port.widget(id, None, None, &response),
        Action::Find => start(state, port),
        Action::Poll => poll(state, port),
        Action::CancelDetect => cancel(state, port),
        Action::Toggle(index, on) => {
            if let Phase::Review { selected, .. } = &mut state.phase
                && let Some(flag) = selected.get_mut(index)
            {
                *flag = on;
            }
        }
        Action::Apply => {
            if let Phase::Review { result, selected } = std::mem::take(&mut state.phase) {
                state.phase = Phase::Applying { result, selected };
            }
        }
        Action::Commit => commit(state, port),
        Action::Close => close(state),
    }
}

/// Submit `media.detect_scenes` for the dialog's clip.
fn start(state: &mut State, port: &mut Port<'_>) {
    let Some(item) = state.item else {
        return;
    };
    let Some(asset) = video_source(port.session, item) else {
        state.phase = Phase::Failed(Text::SceneSplitRefused);
        return;
    };
    if state.runner.is_none() {
        state.runner = job_runner(port.read.budget);
    }
    let Some(runner) = state.runner.as_mut() else {
        state.phase = Phase::Failed(Text::SceneSplitRefused);
        return;
    };
    let job = match &state.source {
        Some(source) => source(runner, asset),
        None => match state.media.as_deref() {
            Some(media) => {
                let command = Command::DetectScenes {
                    asset,
                    settings: DetectionSettings::default(),
                };
                match control(port, Some(media), runner, command) {
                    Ok(Ok(ControlResult::JobAccepted { job, .. })) => Some(job),
                    _ => None,
                }
            }
            None => None,
        },
    };
    state.phase = match job {
        Some(job) => Phase::Detecting {
            job,
            progress: 0.0,
            cancelling: false,
        },
        None => Phase::Failed(Text::SceneSplitRefused),
    };
}

/// Advance the runner and read the job's state; the result becomes the review.
fn poll(state: &mut State, port: &mut Port<'_>) {
    let Phase::Detecting { job, .. } = &state.phase else {
        return;
    };
    let job = *job;
    let Some(runner) = state.runner.as_mut() else {
        state.phase = Phase::Failed(Text::SceneSplitFailed);
        return;
    };
    if runner.tick().is_err() {
        state.phase = Phase::Failed(Text::SceneSplitFailed);
        return;
    }
    match control(
        port,
        state.media.as_deref(),
        runner,
        Command::JobStatus { job },
    ) {
        Ok(Ok(ControlResult::Job { snapshot })) => match snapshot.state {
            JobState::Completed => {
                state.phase = match snapshot.output {
                    Some(JobOutput::Scenes(result)) if result.candidates.is_empty() => {
                        Phase::NoCuts
                    }
                    Some(JobOutput::Scenes(result)) => {
                        let selected = vec![true; result.candidates.len()];
                        Phase::Review { result, selected }
                    }
                    _ => Phase::Failed(Text::SceneSplitFailed),
                };
            }
            JobState::Cancelled => state.phase = Phase::Cancelled,
            JobState::Failed => state.phase = Phase::Failed(Text::SceneSplitFailed),
            _ => {
                if let Phase::Detecting { progress, .. } = &mut state.phase {
                    *progress = snapshot.progress;
                }
            }
        },
        _ => state.phase = Phase::Failed(Text::SceneSplitFailed),
    }
}

/// Ask the job to stop; the next poll reports it cancelled and the timeline is untouched.
fn cancel(state: &mut State, port: &mut Port<'_>) {
    let Phase::Detecting {
        job, cancelling, ..
    } = &mut state.phase
    else {
        return;
    };
    *cancelling = true;
    let job = *job;
    let Some(runner) = state.runner.as_mut() else {
        return;
    };
    let _ = control(
        port,
        state.media.as_deref(),
        runner,
        Command::JobCancel { job },
    );
}

/// Split the reviewed cuts through the engine's reviewed-cut apply, one undo step.
fn commit(state: &mut State, port: &mut Port<'_>) {
    let Phase::Applying { result, selected } = std::mem::take(&mut state.phase) else {
        return;
    };
    let Some(item) = state.item else {
        return;
    };
    let indexes: Vec<usize> = selected
        .iter()
        .enumerate()
        .filter_map(|(index, on)| on.then_some(index))
        .collect();
    let applied = port.with_session(|session| {
        apply_reviewed(session, item, &result, &indexes)
            .map_err(|_| crate::Error::Invalid("reviewed scene cuts"))
    });
    match applied {
        Ok(()) => {
            state.open = false;
            port.sync();
            let added = i18n::text(port.locale(), Text::SceneSplitDone);
            port.toast(added, None);
        }
        Err(_) => state.phase = Phase::Failed(Text::SceneSplitApplyFailed),
    }
}

/// Closing a running detection cancels it; the dialog keeps no candidates.
fn close(state: &mut State) {
    if let (Phase::Detecting { job, .. }, Some(runner)) = (&state.phase, state.runner.as_mut()) {
        let _ = runner.cancel_job(*job);
    }
    state.open = false;
    state.phase = Phase::Idle;
}

/// One control command in the editor session, with the desktop's trusted caller.
fn control(
    port: &mut Port<'_>,
    media: Option<&Mutex<MediaAccess>>,
    runner: &mut JobRunner,
    command: Command,
) -> crate::Result<engine::control::Result<ControlResult>> {
    port.with_session(|session| {
        Ok(session.control_command(
            command,
            Caller {
                principal: "vibecut-scene-split",
                external: false,
                policy: &ALLOW,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media,
                chapters: None,
                captions: None,
                vision: None,
            },
            runner,
        ))
    })
}

/// The runner the detection job runs on; one job at a time, no network.
fn job_runner(budget: &ByteBudget) -> Option<JobRunner> {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(20),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )
    .ok()
}

/// The source of the video clip `item`, or `None` for any other item.
fn video_source(session: &Session, item: ItemId) -> Option<AssetId> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|candidate| candidate.id == item)
        .filter(|candidate| matches!(candidate.content, ItemParams::Video(_)))
        .and_then(|candidate| candidate.source_asset)
}

/// A candidate time in seconds, two decimals.
fn seconds(at: Tick) -> String {
    format!("{:.2} s", at.0 as f64 / TICKS_PER_SECOND as f64)
}

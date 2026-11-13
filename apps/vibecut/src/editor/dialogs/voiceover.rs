//! Records one voice-over take and inserts it at the playhead.
use crate::{
    editor::{Command, Context, CreatedId, Desk, ItemParams, NewItem, SourceAsset, Tick, egui},
    i18n::{self, Text},
    services::voiceover::{CpalHost, Failure, Finished, InputHost, Recorder},
};
use media::AssetId;
use std::{path::PathBuf, sync::Arc, time::Duration};
use time::SourceRange;

/// Automation id and widget salt of the microphone chooser.
const DEVICE_ID: &str = "voiceover.device";
/// Meter floor: the same display floor the timeline waveform uses.
const METER_FLOOR_DB: f32 = -40.0;
/// How much of the previous meter reading survives one frame.
const METER_DECAY: f32 = 0.85;

/// A running take and the playhead it started at.
struct Take {
    recorder: Recorder,
    at: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    Capture(Failure),
    Insert,
}

impl Problem {
    fn text(self) -> Text {
        match self {
            Self::Capture(Failure::PermissionDenied) => Text::VoiceoverPermission,
            Self::Capture(Failure::NoDevice) => Text::VoiceoverNoDevice,
            Self::Capture(Failure::Unavailable) => Text::VoiceoverUnavailable,
            Self::Capture(Failure::Unsupported) => Text::VoiceoverUnsupported,
            Self::Capture(Failure::Overrun) => Text::VoiceoverOverrun,
            Self::Capture(Failure::Storage) => Text::VoiceoverStorage,
            Self::Capture(Failure::Encode) => Text::VoiceoverEncode,
            Self::Capture(Failure::Empty) => Text::VoiceoverEmpty,
            Self::Insert => Text::VoiceoverInsertFailed,
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Capture(Failure::PermissionDenied) => "permission_denied",
            Self::Capture(Failure::NoDevice) => "no_device",
            Self::Capture(Failure::Unavailable) => "unavailable",
            Self::Capture(Failure::Unsupported) => "unsupported",
            Self::Capture(Failure::Overrun) => "overrun",
            Self::Capture(Failure::Storage) => "storage",
            Self::Capture(Failure::Encode) => "encode",
            Self::Capture(Failure::Empty) => "empty",
            Self::Insert => "insert",
        }
    }
}

/// The dialog's state across frames. A running take is cancelled when it drops.
#[derive(Default)]
pub(in crate::editor) struct State {
    open: bool,
    host: Option<Arc<dyn InputHost>>,
    directory: Option<PathBuf>,
    devices: Vec<String>,
    device: Option<String>,
    take: Option<Take>,
    level: f32,
    problem: Option<Problem>,
}

/// What the dialog shows, for inspection and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceoverView {
    pub open: bool,
    pub recording: bool,
    pub seconds: f64,
    /// Meter fill, 0.0 to 1.0.
    pub level: f32,
    pub devices: Vec<String>,
    pub device: Option<String>,
    pub problem: Option<&'static str>,
}

pub(in crate::editor) enum Action {
    Remember(String, egui::Response),
    Start,
    Stop,
    Cancel,
    Close,
}

impl State {
    /// Inject the input host and the folder takes are written to.
    pub(in crate::editor) fn set(&mut self, host: Arc<dyn InputHost>, directory: PathBuf) {
        self.host = Some(host);
        self.directory = Some(directory);
    }

    pub(in crate::editor) fn open(&mut self) {
        if self.open {
            return;
        }
        self.open = true;
        self.problem = None;
        self.level = 0.0;
        match self.host().devices() {
            Ok(devices) => {
                if devices.is_empty() {
                    self.problem = Some(Problem::Capture(Failure::NoDevice));
                }
                self.devices = devices;
            }
            Err(failure) => {
                self.devices.clear();
                self.problem = Some(Problem::Capture(failure));
            }
        }
        if self
            .device
            .as_ref()
            .is_some_and(|device| !self.devices.contains(device))
        {
            self.device = None;
        }
    }

    pub(in crate::editor) fn is_open(&self) -> bool {
        self.open
    }

    /// A take is running.
    pub(in crate::editor) fn recording(&self) -> bool {
        self.take.is_some()
    }

    pub(in crate::editor) fn view(&self) -> VoiceoverView {
        let seconds = self.take.as_ref().map_or(0.0, |take| {
            take.recorder.frames() as f64 / f64::from(take.recorder.sample_rate())
        });
        VoiceoverView {
            open: self.open,
            recording: self.take.is_some(),
            seconds,
            level: meter_fraction(self.level),
            devices: self.devices.clone(),
            device: self.device.clone(),
            problem: self.problem.map(Problem::code),
        }
    }

    fn host(&mut self) -> Arc<dyn InputHost> {
        self.host.get_or_insert_with(|| Arc::new(CpalHost)).clone()
    }

    /// Read the meter and react to what the device, queue or worker reported.
    fn poll(&mut self, actions: &mut Vec<Action>) {
        let Some(take) = &self.take else {
            return;
        };
        self.level = take.recorder.level().max(self.level * METER_DECAY);
        if let Some(failure) = take.recorder.failure() {
            // Dropping the recorder joins the worker and deletes the unfinished file.
            self.take = None;
            self.level = 0.0;
            self.problem = Some(Problem::Capture(failure));
        } else if take.recorder.full() {
            actions.push(Action::Stop);
        }
    }
}

fn meter_fraction(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    ((20.0 * level.log10() - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

fn clock(seconds: f64) -> String {
    format!("{}:{:04.1}", (seconds / 60.0).floor(), seconds % 60.0)
}

pub(super) fn show(state: &mut State, cx: &Context<'_>, ctx: &egui::Context) -> Vec<Action> {
    let mut actions = Vec::new();
    if !state.open {
        return actions;
    }
    state.poll(&mut actions);
    let tr = |text: Text| text.text(cx.locale);
    let recording = state.take.is_some();
    let modal = egui::Modal::new(egui::Id::unique("voiceover.dialog"))
        .frame(crate::kit::dialog_frame(ctx))
        .show(ctx, |ui| {
            crate::kit::dialog_title(ui, tr(Text::ActionVoiceoverOpen));
            ui.horizontal(|ui| {
                ui.label(tr(Text::VoiceoverDevice));
                ui.add_enabled_ui(!recording, |ui| {
                    let shown = state
                        .device
                        .clone()
                        .unwrap_or_else(|| tr(Text::VoiceoverDefaultInput).to_owned());
                    let combo = egui::ComboBox::from_id_salt(DEVICE_ID)
                        .selected_text(&shown)
                        .show_ui(ui, |ui| {
                            let response = ui.selectable_label(
                                state.device.is_none(),
                                tr(Text::VoiceoverDefaultInput),
                            );
                            actions.push(Action::Remember(
                                "voiceover.device.default".into(),
                                response.clone(),
                            ));
                            if response.clicked() {
                                state.device = None;
                            }
                            for (index, name) in state.devices.clone().into_iter().enumerate() {
                                let response = ui.selectable_label(
                                    state.device.as_deref() == Some(name.as_str()),
                                    name.as_str(),
                                );
                                actions.push(Action::Remember(
                                    format!("voiceover.device.{index}"),
                                    response.clone(),
                                ));
                                if response.clicked() {
                                    state.device = Some(name);
                                }
                            }
                        });
                    actions.push(Action::Remember(DEVICE_ID.into(), combo.response));
                });
            });
            ui.horizontal(|ui| {
                ui.label(tr(Text::VoiceoverLevel));
                let meter = ui.add(
                    egui::ProgressBar::new(meter_fraction(state.level))
                        .desired_width(ui.spacing().slider_width),
                );
                actions.push(Action::Remember("voiceover.meter".into(), meter));
            });
            if let Some(take) = &state.take {
                let seconds =
                    take.recorder.frames() as f64 / f64::from(take.recorder.sample_rate());
                let time = clock(seconds);
                let status = i18n::format(cx.locale, Text::VoiceoverRecording, &[("time", &time)]);
                let label = ui.label(&status);
                // A running status is read by name, not only as a bare value.
                ctx.accesskit_node_builder(label.id, |node| node.set_label(status));
                actions.push(Action::Remember("voiceover.time".into(), label));
            }
            if let Some(problem) = state.problem {
                let label = ui.colored_label(crate::kit::error_text(cx.dark), tr(problem.text()));
                actions.push(Action::Remember("voiceover.problem".into(), label));
            }
            ui.horizontal(|ui| {
                if recording {
                    let stop = ui.button(tr(Text::VoiceoverStop));
                    actions.push(Action::Remember("voiceover.stop".into(), stop.clone()));
                    if stop.clicked() {
                        actions.push(Action::Stop);
                    }
                    let cancel = ui.button(tr(Text::Cancel));
                    actions.push(Action::Remember("voiceover.cancel".into(), cancel.clone()));
                    if cancel.clicked() {
                        actions.push(Action::Cancel);
                    }
                } else {
                    let record = ui.button(tr(Text::VoiceoverRecord));
                    actions.push(Action::Remember("voiceover.record".into(), record.clone()));
                    if record.clicked() {
                        actions.push(Action::Start);
                    }
                    let close = ui.button(tr(Text::VoiceoverClose));
                    actions.push(Action::Remember("voiceover.close".into(), close.clone()));
                    if close.clicked() {
                        actions.push(Action::Close);
                    }
                }
            });
        });
    if modal.should_close() {
        actions.push(Action::Close);
    }
    if recording {
        ctx.request_repaint_after(Duration::from_millis(33));
    }
    actions
}

pub(super) fn apply(desk: &mut Desk<'_>, action: Action) {
    match action {
        Action::Remember(id, response) => desk.editor().remember(id, &response),
        Action::Start => start(desk),
        Action::Stop => stop(desk),
        Action::Cancel => {
            let state = &mut desk.editor().dialogs.voiceover;
            state.take = None;
            state.level = 0.0;
        }
        Action::Close => {
            let state = &mut desk.editor().dialogs.voiceover;
            state.take = None;
            state.level = 0.0;
            state.open = false;
        }
    }
}

fn start(desk: &mut Desk<'_>) {
    let editor = desk.editor();
    if editor.dialogs.voiceover.take.is_some() {
        return;
    }
    let at = editor.playback.position().unwrap_or(Tick::ZERO);
    let directory = editor
        .dialogs
        .voiceover
        .directory
        .clone()
        .unwrap_or_else(|| project::media_dir(editor.session.saved_path()));
    let state = &mut editor.dialogs.voiceover;
    let host = state.host();
    state.problem = None;
    match Recorder::start(host.as_ref(), state.device.as_deref(), &directory) {
        Ok(recorder) => {
            state.level = 0.0;
            state.take = Some(Take { recorder, at });
        }
        Err(failure) => state.problem = Some(Problem::Capture(failure)),
    }
}

fn stop(desk: &mut Desk<'_>) {
    let state = &mut desk.editor().dialogs.voiceover;
    let Some(Take { recorder, at }) = state.take.take() else {
        return;
    };
    state.level = 0.0;
    let finished = match recorder.stop() {
        Ok(finished) => finished,
        Err(failure) => {
            state.problem = Some(Problem::Capture(failure));
            return;
        }
    };
    if insert(desk, &finished, at).is_ok() {
        desk.editor().dialogs.voiceover.open = false;
        let added = i18n::text(desk.editor().preferences.locale, Text::VoiceoverAdded);
        desk.toast(added, None);
    } else {
        // The asset never reached the project, so the promoted file is an orphan.
        let _ = std::fs::remove_file(&finished.path);
        desk.editor().dialogs.voiceover.problem = Some(Problem::Insert);
    }
}

/// Register the take as an asset and place it at `at`, as one undo step.
fn insert(desk: &mut Desk<'_>, finished: &Finished, at: Tick) -> Result<(), Problem> {
    let next = desk
        .editor()
        .session
        .project()
        .data()
        .assets
        .iter()
        .map(|asset| asset.id().0)
        .max()
        .map_or(1, |id| id.saturating_add(1));
    let asset =
        SourceAsset::import(AssetId(next), finished.path.clone()).map_err(|_| Problem::Insert)?;
    let duration = asset
        .probe()
        .duration
        .filter(|duration| *duration > Tick::ZERO)
        .ok_or(Problem::Insert)?;
    let source_range = SourceRange::new(Tick::ZERO, duration).map_err(|_| Problem::Insert)?;
    let item = NewItem {
        retime: Default::default(),
        animations: Default::default(),
        source_asset: Some(asset.id()),
        source_range,
        start: at,
        content: ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
            cleanup: Default::default(),
        }),
    };
    let created = desk
        .editor()
        .session
        .transaction(|session| -> std::result::Result<_, engine::EditError> {
            session.apply(Command::AddAsset { asset })?;
            let outcome = session.apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item,
            })?;
            Ok(outcome.created)
        })
        .map_err(|_| Problem::Insert)?;
    let items = created
        .into_iter()
        .filter_map(|id| match id {
            CreatedId::Item(item) => Some(item),
            _ => None,
        })
        .collect();
    desk.select(items);
    desk.sync();
    Ok(())
}

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
//! Voice-over recording through a scripted input host: no microphone, no native app.
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use project::{Project, ProjectId};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU16, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use time::{TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, VoiceoverView, egui,
    i18n::{Locale, Text},
    preferences::Preferences,
    services::{
        playback::{Files, Playback},
        voiceover::{Failure, InputHost, Opened, QUEUE_BLOCKS, Recorder, Sink},
    },
};

/// The input device seam: the test plays the audio callback.
struct Script {
    devices: Mutex<Result<Vec<String>, Failure>>,
    refusal: Mutex<Option<Failure>>,
    sink: Arc<Mutex<Option<Sink>>>,
    opened: Mutex<Vec<Option<String>>>,
    rate: AtomicU32,
    channels: AtomicU16,
}

/// Dropping it ends the capture, as dropping a native stream does.
struct Capture(Arc<Mutex<Option<Sink>>>);
impl Drop for Capture {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = None;
    }
}

impl InputHost for Script {
    fn devices(&self) -> Result<Vec<String>, Failure> {
        self.devices.lock().unwrap().clone()
    }
    fn open(&self, device: Option<&str>, sink: Sink) -> Result<Opened, Failure> {
        self.opened.lock().unwrap().push(device.map(str::to_owned));
        if let Some(failure) = *self.refusal.lock().unwrap() {
            return Err(failure);
        }
        *self.sink.lock().unwrap() = Some(sink);
        Ok(Opened {
            sample_rate: self.rate.load(Ordering::SeqCst),
            channels: self.channels.load(Ordering::SeqCst),
            capture: Box::new(Capture(self.sink.clone())),
        })
    }
}

impl Script {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::new(Ok(vec!["Mic A".into(), "Mic B".into()])),
            refusal: Mutex::new(None),
            sink: Arc::new(Mutex::new(None)),
            opened: Mutex::new(Vec::new()),
            rate: AtomicU32::new(48_000),
            channels: AtomicU16::new(1),
        })
    }
    fn format(&self, rate: u32, channels: u16) {
        self.rate.store(rate, Ordering::SeqCst);
        self.channels.store(channels, Ordering::SeqCst);
    }
    /// Deliver `frames` of a 440 Hz tone at `amplitude`, as one callback.
    fn tone(&self, frames: usize, amplitude: f32) -> bool {
        let rate = self.rate.load(Ordering::SeqCst) as f32;
        let channels = usize::from(self.channels.load(Ordering::SeqCst));
        let samples: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let x = amplitude * (std::f32::consts::TAU * 440.0 * i as f32 / rate).sin();
                std::iter::repeat_n(x, channels)
            })
            .collect();
        match self.sink.lock().unwrap().as_ref() {
            Some(sink) => {
                sink.push(&samples, channels as u16);
                true
            }
            None => false,
        }
    }
    fn fail(&self, failure: Failure) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink.fail(failure);
        }
    }
    fn capturing(&self) -> bool {
        self.sink.lock().unwrap().is_some()
    }
}

fn editor(script: &Arc<Script>, directory: &Path) -> Editor {
    let session = Session::new(Project::empty(ProjectId(86), "Voice".into()).unwrap(), 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.set_voiceover(script.clone(), directory.to_owned());
    editor
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn view(harness: &Harness<Editor>) -> VoiceoverView {
    harness.state().inspect().unwrap().voiceover
}

fn wait(harness: &mut Harness<Editor>, what: &str, mut predicate: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(1);
        if predicate(harness.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            view(harness)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn files(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Every audio item as (start, duration) ticks.
fn audio_items(editor: &Editor) -> Vec<(i64, i64)> {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .filter(|item| matches!(item.content, project::ItemParams::Audio(_)))
        .map(|item| (item.start.0, item.duration().unwrap().0))
        .collect()
}

fn start_recording(harness: &mut Harness<Editor>) {
    click(harness, "voiceover.open");
    assert!(view(harness).open);
    click(harness, "voiceover.record");
    assert!(view(harness).recording, "{:?}", view(harness));
}

/// Slack for AAC priming and trailing padding: two 1024-sample frames at the slowest
/// rate used here. A wall-clock duration would miss the sample count by far more.
const AAC_FRAME_TICKS: i64 = TICKS_PER_SECOND * 2048 / 44_100;

#[test]
fn a_take_is_promoted_and_inserted_at_the_playhead_as_one_undo_step() {
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("media");
    let script = Script::new();
    let mut harness = build(editor(&script, &media));
    harness.run_steps(2);

    // Open from the timeline toolbar; devices come from the host, default first.
    click(&mut harness, "voiceover.open");
    let opened = view(&harness);
    assert!(opened.open && !opened.recording && opened.problem.is_none());
    assert_eq!(opened.devices, ["Mic A", "Mic B"]);
    click(&mut harness, "voiceover.record");
    assert_eq!(*script.opened.lock().unwrap(), [None], "default input");

    // Live meter and sample-count time while recording; the take is not promoted yet.
    for _ in 0..15 {
        assert!(script.tone(4800, 0.5));
    }
    wait(&mut harness, "1.5 s encoded", |editor| {
        let view = editor.inspect().unwrap().voiceover;
        view.recording && view.seconds >= 1.5 - 1e-9
    });
    assert!(view(&harness).level > 0.5, "-6 dBFS reads above half scale");
    assert_eq!(
        files(&media),
        ["voiceover-1.m4a.part"],
        "no partial promotion"
    );
    assert_eq!(harness.state().session.history_len(), (0, 0));

    click(&mut harness, "voiceover.stop");
    wait(&mut harness, "dialog closes", |editor| {
        !editor.inspect().unwrap().voiceover.open
    });
    assert!(!script.capturing(), "the device was released");
    assert_eq!(files(&media), ["voiceover-1.m4a"]);

    // One audio item at the playhead whose length is the captured sample count.
    let items = audio_items(harness.state());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].0, 0, "inserted at the playhead");
    let expected = TICKS_PER_SECOND * 3 / 2;
    assert!(
        (items[0].1 - expected).abs() <= AAC_FRAME_TICKS,
        "{} ticks for 1.5 s of samples",
        items[0].1
    );
    let state = harness.state();
    assert_eq!(state.session.project().data().assets.len(), 1);
    assert_eq!(
        state.session.project().data().assets[0].path(),
        media.join("voiceover-1.m4a").canonicalize().unwrap()
    );
    assert_eq!(state.session.history_len(), (1, 0), "one undo step");
    assert_eq!(state.session.selection().len(), 1, "the take is selected");

    // Undo removes the item and its asset; the file stays; redo restores both.
    harness.state_mut().activate("edit.undo").unwrap();
    assert!(audio_items(harness.state()).is_empty());
    assert!(harness.state().session.project().data().assets.is_empty());
    assert_eq!(files(&media), ["voiceover-1.m4a"]);
    harness.state_mut().activate("edit.redo").unwrap();
    assert_eq!(audio_items(harness.state()).len(), 1);
    assert_eq!(harness.state().session.project().data().assets.len(), 1);

    // A stereo 44.1 kHz take from a chosen device starts at the new playhead.
    let half = Tick(TICKS_PER_SECOND / 2);
    let project = harness.state().session.project().clone();
    harness
        .state_mut()
        .playback
        .seek(half, false, project)
        .unwrap();
    script.format(44_100, 2);
    click(&mut harness, "voiceover.open");
    click(&mut harness, "voiceover.device");
    click(&mut harness, "voiceover.device.1");
    assert_eq!(view(&harness).device.as_deref(), Some("Mic B"));
    click(&mut harness, "voiceover.record");
    assert_eq!(
        script.opened.lock().unwrap().last().unwrap().as_deref(),
        Some("Mic B")
    );
    for _ in 0..10 {
        assert!(script.tone(4410, 0.25));
    }
    click(&mut harness, "voiceover.stop");
    wait(&mut harness, "second take", |editor| {
        audio_items(editor).len() == 2
    });
    let second = audio_items(harness.state())
        .into_iter()
        .find(|(start, _)| *start == half.0)
        .expect("second take starts at the playhead");
    assert!(
        (second.1 - TICKS_PER_SECOND).abs() <= AAC_FRAME_TICKS,
        "{} ticks for 1 s of samples",
        second.1
    );
    assert_eq!(files(&media), ["voiceover-1.m4a", "voiceover-2.m4a"]);
    assert_eq!(harness.state().session.history_len(), (2, 0));
}

#[test]
fn cancel_and_every_failure_leave_no_file_and_no_item() {
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("media");
    let script = Script::new();
    let mut harness = build(editor(&script, &media));
    harness.run_steps(2);

    // Cancel: the device is released, the unfinished file is gone, the dialog stays.
    start_recording(&mut harness);
    assert!(script.tone(4800, 0.5));
    click(&mut harness, "voiceover.cancel");
    let after = view(&harness);
    assert!(after.open && !after.recording && after.problem.is_none());
    assert!(!script.capturing());
    assert!(files(&media).is_empty());

    // The device drops out mid-take: typed failure, take discarded.
    click(&mut harness, "voiceover.record");
    assert!(script.tone(4800, 0.5));
    script.fail(Failure::Unavailable);
    wait(&mut harness, "device failure", |editor| {
        editor.inspect().unwrap().voiceover.problem == Some("unavailable")
    });
    assert!(!view(&harness).recording);
    assert!(!script.capturing());
    assert!(files(&media).is_empty());

    // Stop with nothing captured: no empty asset.
    click(&mut harness, "voiceover.record");
    click(&mut harness, "voiceover.stop");
    assert_eq!(view(&harness).problem, Some("empty"));
    assert!(files(&media).is_empty());

    // Refusals at open: permission denied, a vanished device, an unusable format.
    for (failure, code) in [
        (Failure::PermissionDenied, "permission_denied"),
        (Failure::NoDevice, "no_device"),
        (Failure::Unsupported, "unsupported"),
    ] {
        *script.refusal.lock().unwrap() = Some(failure);
        click(&mut harness, "voiceover.record");
        assert_eq!(view(&harness).problem, Some(code));
        assert!(!view(&harness).recording && !script.capturing());
    }
    *script.refusal.lock().unwrap() = None;

    // No input device at all is reported when the dialog opens.
    click(&mut harness, "voiceover.close");
    *script.devices.lock().unwrap() = Ok(Vec::new());
    click(&mut harness, "voiceover.open");
    assert_eq!(view(&harness).problem, Some("no_device"));
    click(&mut harness, "voiceover.close");

    // A media folder that cannot be created is a typed disk failure.
    *script.devices.lock().unwrap() = Ok(vec!["Mic A".into()]);
    let blocked = root.path().join("blocked");
    std::fs::write(&blocked, b"file").unwrap();
    harness
        .state_mut()
        .set_voiceover(script.clone(), blocked.join("media"));
    click(&mut harness, "voiceover.open");
    click(&mut harness, "voiceover.record");
    assert_eq!(view(&harness).problem, Some("storage"));
    assert!(!script.capturing());

    // Through all of it the project never changed.
    let state = harness.state();
    assert_eq!(state.session.history_len(), (0, 0));
    assert!(state.session.project().data().assets.is_empty());
    assert!(audio_items(state).is_empty());
}

#[test]
fn a_producer_that_outruns_the_encoder_fails_with_bounded_memory_and_joins_the_worker() {
    let root = tempfile::tempdir().unwrap();
    let script = Script::new();
    let recorder = Recorder::start(script.as_ref(), None, root.path()).unwrap();
    // 16384-sample blocks arrive far faster than AAC encodes them; the queue holds
    // QUEUE_BLOCKS of them at most, so the surplus is a typed overrun, never growth.
    let mut sent = 0;
    while recorder.failure().is_none() && sent < QUEUE_BLOCKS * 200 {
        assert!(script.tone(16_384, 0.5));
        sent += 1;
    }
    assert_eq!(recorder.failure(), Some(Failure::Overrun));
    assert!(sent < QUEUE_BLOCKS * 200, "the burst hit the queue bound");
    recorder.cancel();
    assert!(!script.capturing(), "cancel releases the device");
    assert!(
        files(root.path()).is_empty(),
        "cancel joined the worker and removed the file"
    );
}

#[test]
fn the_dialog_reads_in_english_and_vietnamese_and_renders_a_recording() {
    let root = tempfile::tempdir().unwrap();
    let script = Script::new();
    let mut editor = editor(&script, &root.path().join("media"));
    editor.preferences.locale = Locale::Vi;
    let mut harness = build(editor);
    harness.run_steps(2);
    start_recording(&mut harness);
    assert!(script.tone(24_000, 0.5));
    wait(&mut harness, "half a second", |editor| {
        editor.inspect().unwrap().voiceover.seconds >= 0.5 - 1e-9
    });
    for locale in [Locale::Vi, Locale::En] {
        harness.state_mut().preferences.locale = locale;
        harness.run_steps(2);
        for (id, text) in [
            ("voiceover.stop", Text::VoiceoverStop),
            ("voiceover.cancel", Text::Cancel),
        ] {
            let label = harness
                .get(By::new().predicate(|n| n.author_id() == Some(id)))
                .accesskit_node()
                .label();
            assert_eq!(
                label.as_deref(),
                Some(text.text(locale)),
                "{id} in {locale:?}"
            );
        }
        harness.get_by_label(Text::VoiceoverLevel.text(locale));
        let time = vibecut::i18n::format(locale, Text::VoiceoverRecording, &[("time", "0:00.5")]);
        let shown = harness
            .get(By::new().predicate(|n| n.author_id() == Some("voiceover.time")))
            .accesskit_node()
            .label();
        assert_eq!(shown.as_deref(), Some(time.as_str()));
    }
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-86-voiceover.png"]
fn voiceover_dialog_renders_a_recording_in_progress() {
    let root = tempfile::tempdir().unwrap();
    let script = Script::new();
    let mut harness = build(editor(&script, &root.path().join("media")));
    harness.run_steps(2);
    start_recording(&mut harness);
    assert!(script.tone(24_000, 0.5));
    wait(&mut harness, "half a second", |editor| {
        editor.inspect().unwrap().voiceover.seconds >= 0.5 - 1e-9
    });
    let image = harness.render().expect("offscreen render");
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-86-voiceover.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

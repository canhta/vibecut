//! Drives the export dialog alone over the one-area bench.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{AlsoOutputs, ExportFormat, ExportOptions, ExportQuality};
use crate::editor::frame::bench::Bench;
use crate::editor::{AppCommand, Editor, egui};
use crate::preferences::{PreferenceStore, Preferences};
use crate::services::playback::{Files, Playback, Sources};
use crate::{Error, i18n::Locale};
use egui_kittest::kittest::Queryable;
use engine::{ByteBudget, Session};
use media::{AssetData, AssetId, ContentHash, SourceAsset, TimeBase};
use project::{ItemParams, Project, ProjectId};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

const SECOND: i64 = TICKS_PER_SECOND;

fn temp_target(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}
fn project(empty: bool, source: bool) -> Project {
    project_named("Export fixture", empty, source)
}
fn project_named(name: &str, empty: bool, source: bool) -> Project {
    let mut d = Project::empty(ProjectId(18), name.into())
        .unwrap()
        .into_data();
    d.canvas.width = 64;
    d.canvas.height = 64;
    let range = SourceRange::new(Tick::ZERO, Tick(SECOND / 10)).unwrap();
    if source {
        d.assets.push(
            testkit::source::asset(1, temp_target("vibecut-export-source-missing.mp4"))
                .format("mp4")
                .duration(range.duration())
                .content_hash(ContentHash::from_bytes(b"source"))
                .stream(
                    testkit::source::video()
                        .time_base(TimeBase::new(1, 30).unwrap())
                        .duration(range.duration())
                        .frame_count(3)
                        .variable_frame_rate(false)
                        .size(64, 64)
                        .build(),
                )
                .build(),
        );
    }
    if !empty {
        d.scene_mut().timeline.main.items.push(
            testkit::item::item(
                project::ItemId(1),
                if source {
                    ItemParams::Video(project::VideoParams::plain())
                } else {
                    ItemParams::Overlay(project::OverlayParams {
                        color: project::Color([100, 150, 200, 255]),
                    })
                },
            )
            .source_asset(source.then_some(AssetId(1)))
            .source(range)
            .build(),
        );
    }
    Project::new(d).unwrap()
}
fn editor(empty: bool, source: bool) -> Editor {
    editor_project(project(empty, source))
}
fn editor_project(project: Project) -> Editor {
    let session = Session::new(project, 10).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(768 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Editor::new(session, playback, Preferences::default())
}
fn bench(editor: Editor) -> Bench<super::super::State, super::super::Action> {
    Bench::hosted(
        editor,
        |editor| std::mem::take(&mut editor.dialogs),
        |editor, state| editor.dialogs = state,
        super::super::show,
        crate::editor::Editor::apply_dialogs,
        egui::vec2(1280.0, 800.0),
    )
}
type DialogBench = Bench<super::super::State, super::super::Action>;
fn enabled(bench: &DialogBench, id: &str) -> bool {
    bench
        .editor()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .find(|w| w.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .enabled
}
fn open(bench: &mut DialogBench) {
    bench.editor_mut().activate("file.export").unwrap();
    bench.harness_mut().run_steps(2);
}
fn wait(bench: &mut DialogBench) -> super::ExportPhase {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        bench.editor_mut().tick();
        bench.step();
        let inspect = bench.editor().export_state().unwrap().inspect();
        if matches!(
            inspect.phase,
            super::ExportPhase::Ready | super::ExportPhase::Failed | super::ExportPhase::Settings
        ) {
            return inspect.phase;
        }
        assert!(Instant::now() < deadline, "export stalled");
        std::thread::yield_now();
    }
}

struct Gate {
    entered: Mutex<mpsc::Sender<()>>,
    release: (Mutex<bool>, Condvar),
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
struct GateFactory(Arc<Gate>);
impl Sources for GateFactory {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Gated(self.0.clone()))
    }
}
struct Gated(Arc<Gate>);
impl render::SourceProvider for Gated {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        self.0.entered.lock().unwrap().send(()).unwrap();
        let mut release = self.0.release.0.lock().unwrap();
        while !*release {
            release = self.0.release.1.wait(release).unwrap();
        }
        if hooks.cancelled() {
            Err(render::Error::Cancelled)
        } else {
            Err(render::Error::Unsupported("injected source failure"))
        }
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: Tick,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        panic!("unexpected source request after injected refusal")
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: i64,
        _: usize,
        _: u32,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        panic!("unexpected source request after injected refusal")
    }
}
struct Restorable {
    restored: Arc<AtomicBool>,
    decoded: Arc<AtomicUsize>,
}
impl Sources for Restorable {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Self {
            restored: self.restored.clone(),
            decoded: self.decoded.clone(),
        })
    }
}
impl render::SourceProvider for Restorable {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok((!self.restored.load(Ordering::SeqCst)).then_some(render::UnavailableReason::Missing))
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        self.decoded.fetch_add(1, Ordering::SeqCst);
        render::Image::new(64, 64, [0.0, 0.0, 0.0, 1.0], hooks).map(Some)
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: i64,
        _: usize,
        _: u32,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        panic!("source audio is disabled in this fixture")
    }
}
fn restored_source(restored: bool) -> (Arc<Restorable>, Arc<AtomicUsize>) {
    let flag = Arc::new(AtomicBool::new(restored));
    let decoded = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Restorable {
            restored: flag,
            decoded: decoded.clone(),
        }),
        decoded,
    )
}

#[test]
fn export_choices_default_persist_and_empty_timeline_refuses() {
    for locale in [Locale::En, Locale::Vi] {
        // The default name follows the UI language: the dialog's Name starts from the project's
        // default name in that language (an English render never shows the Vietnamese one).
        let name = crate::i18n::text(locale, crate::i18n::Text::UntitledProject);
        assert_eq!(
            name,
            match locale {
                Locale::En => "Untitled Project",
                Locale::Vi => "Dự án chưa đặt tên",
            }
        );
        let mut e = editor_project(project_named(name, true, false));
        e.preferences.locale = locale;
        assert_eq!(e.session.project().data().name.as_str(), name);
        assert_eq!(e.preferences.export, ExportOptions::default());
        let mut bench = bench(e);
        open(&mut bench);
        let state = bench.editor().export_state().unwrap().inspect();
        assert!(state.open && state.empty);
        assert!(!enabled(&bench, "export.run"));
        bench.click("export.format");
        bench.click("export.format.webm");
        bench.click("export.bitrate");
        bench.click("export.bitrate.higher");
        // Audio beside Video is a second file: an undoable preference edit.
        bench.click("export.audio.include");
        assert_eq!(
            bench.editor().inspect().unwrap().export_options,
            ExportOptions {
                format: ExportFormat::Webm,
                quality: ExportQuality::Higher,
                also: AlsoOutputs {
                    audio: Some(ExportFormat::M4a),
                    ..Default::default()
                },
                ..Default::default()
            }
        );
        bench.editor_mut().activate("edit.undo").unwrap();
        assert_eq!(bench.editor().preferences.export.also.audio, None);
        bench.editor_mut().activate("edit.redo").unwrap();
        assert_eq!(
            bench.editor().preferences.export.also.audio,
            Some(ExportFormat::M4a)
        );
        // Audio-only is Audio ticked alone: Video is unticked, its block greys (listed, not
        // hidden) and the Audio Format row stays alive.
        bench.click("export.video.include");
        assert!(
            bench
                .editor()
                .inspect()
                .unwrap()
                .export_options
                .format
                .audio_only()
        );
        bench.harness_mut().run_steps(2);
        for id in [
            "export.resolution",
            "export.bitrate",
            "export.codec",
            "export.format",
            "export.fps",
            "export.profile",
            "export.custom.size",
            "export.audio.include",
        ] {
            assert!(!enabled(&bench, id), "{id} applies to audio-only output");
        }
        assert!(
            enabled(&bench, "export.video.include"),
            "unticked Video can be ticked again"
        );
        assert!(enabled(&bench, "export.audio.format"));
        bench.click("export.audio.format");
        bench.click("export.audio.format.opus");
        assert_eq!(
            bench.editor().preferences.export.format,
            ExportFormat::AudioWebm
        );
        // WAV and FLAC have built-in encoders, so they are always listed; MP3 follows
        // the registry.
        for (format, id) in [
            (ExportFormat::Wav, "export.audio.format.wav"),
            (ExportFormat::Flac, "export.audio.format.flac"),
        ] {
            bench.click("export.audio.format");
            bench.click(id);
            assert_eq!(bench.editor().preferences.export.format, format);
        }
        if media::audio_export_container_available(media::Container::Mp3) {
            bench.click("export.audio.format");
            bench.click("export.audio.format.mp3");
            assert_eq!(bench.editor().preferences.export.format, ExportFormat::Mp3);
        }
        bench.click("export.audio.format");
        bench.click("export.audio.format.m4a");
        assert_eq!(bench.editor().preferences.export.format, ExportFormat::M4a);
        // Ticking Video again returns to the video format it left and keeps Audio as a
        // second file; unticking Audio greys its format row.
        bench.click("export.video.include");
        bench.harness_mut().run_steps(2);
        let both = bench.editor().preferences.export;
        assert_eq!(both.format, ExportFormat::Webm);
        assert_eq!(both.also.audio, Some(ExportFormat::M4a));
        assert!(enabled(&bench, "export.resolution"));
        assert!(enabled(&bench, "export.audio.format"));
        bench.click("export.audio.include");
        bench.harness_mut().run_steps(2);
        assert_eq!(bench.editor().preferences.export.also.audio, None);
        assert!(!enabled(&bench, "export.audio.format"));
        // Bit rate presets and Custom: Custom starts at 12000 Kbps and shows the Kbps field;
        // a preset clears it again.
        bench.click("export.bitrate");
        bench.click("export.bitrate.custom");
        assert_eq!(
            bench.editor().preferences.export.output.bit_rate,
            Some(12_000_000)
        );
        bench.harness_mut().run_steps(2);
        assert!(enabled(&bench, "export.bitrate.kbps"));
        bench.click("export.bitrate");
        bench.click("export.bitrate.lower");
        assert_eq!(bench.editor().preferences.export.output.bit_rate, None);
        assert_eq!(
            bench.editor().preferences.export.quality,
            ExportQuality::Lower
        );
        bench.click("export.bitrate");
        bench.click("export.bitrate.recommended");
        // GIF is its own section with Resolution only. Ticked beside Video it is a second
        // file at 240P and Video keeps its rate; unticking Video leaves GIF alone: the rate
        // clears, and the Video block greys.
        bench.editor_mut().preferences.export.output.bit_rate = Some(100_000);
        bench.click("export.gif.include");
        bench.harness_mut().run_steps(2);
        let beside = bench.editor().preferences.export;
        assert_eq!(beside.format, ExportFormat::Webm);
        assert_eq!(beside.output.bit_rate, Some(100_000));
        assert_eq!(beside.output.resolution, None);
        assert_eq!(beside.also.gif, Some([240, 240]));
        assert!(enabled(&bench, "export.gif.resolution"));
        bench.click("export.video.include");
        bench.harness_mut().run_steps(2);
        let gif = bench.editor().preferences.export;
        assert_eq!((gif.format, gif.output.bit_rate), (ExportFormat::Gif, None));
        assert_eq!(gif.output.gif, media::GifOptions::default());
        assert_eq!(gif.output.resolution, Some([240, 240]));
        assert_eq!(gif.also, AlsoOutputs::default());
        assert!(enabled(&bench, "export.gif.resolution"));
        assert!(!enabled(&bench, "export.gif.include"));
        for id in [
            "export.resolution",
            "export.bitrate",
            "export.codec",
            "export.format",
            "export.profile",
        ] {
            assert!(!enabled(&bench, id), "{id} applies to GIF");
        }
        // GIF's rate is capped, so the frame rate stays live; Audio and Video switch sections.
        assert!(enabled(&bench, "export.fps"));
        assert!(enabled(&bench, "export.audio.include"));
        assert!(enabled(&bench, "export.video.include"));
        bench.click("export.gif.resolution");
        bench.click("export.gif.resolution.320");
        assert_eq!(
            bench.editor().preferences.export.output.resolution,
            Some([320, 320])
        );
        // Ticking Video again restores the project size and the video format and keeps the
        // GIF as a second file at the size it had; unticking GIF greys its Resolution.
        bench.click("export.video.include");
        bench.harness_mut().run_steps(2);
        let video = bench.editor().preferences.export;
        assert_eq!(video.format, ExportFormat::Webm);
        assert_eq!(video.output.resolution, None);
        assert_eq!(video.also.gif, Some([320, 320]));
        assert!(enabled(&bench, "export.gif.resolution"));
        bench.click("export.gif.include");
        bench.harness_mut().run_steps(2);
        assert_eq!(bench.editor().preferences.export.also.gif, None);
        assert!(!enabled(&bench, "export.gif.resolution"));
        // A PNG sequence has no codec rate: the same bitrate row is disabled and cleared, and
        // the finished output is named without an extension. Codec, profile and the all-scenes
        // choice are reset to the values the export accepts.
        bench.click("export.format");
        bench.click("export.format.mp4");
        bench.click("export.codec");
        bench.click("export.codec.h264");
        bench.editor_mut().preferences.export.output.bit_rate = Some(100_000);
        bench.editor_mut().preferences.export.output.profile = media::H264Profile::High;
        bench.click("export.scene");
        bench.click("export.scene.all");
        bench.click("export.format");
        bench.click("export.format.png_sequence");
        bench.harness_mut().run_steps(2);
        let sequence = bench.editor().preferences.export;
        assert_eq!(
            (sequence.format, sequence.output.bit_rate),
            (ExportFormat::PngSequence, None)
        );
        assert!(sequence.output.codec == render::VideoCodec::Automatic);
        assert!(sequence.output.profile == media::H264Profile::Automatic);
        assert!(sequence.output.scene == render::SceneChoice::Active);
        for id in [
            "export.bitrate",
            "export.codec",
            "export.profile",
            "export.audio.include",
            "export.video.include",
        ] {
            assert!(!enabled(&bench, id), "{id} applies to a PNG sequence");
        }
        // The scene rows exist only while the scene list is open.
        bench.click("export.scene");
        assert!(!enabled(&bench, "export.scene.all"));
        bench.click("export.scene");
        assert_eq!(
            ExportFormat::PngSequence.file_name("My: Project?"),
            "My_ Project_"
        );
        assert_eq!(
            ExportFormat::Mp4.file_name("My: Project?"),
            "My_ Project_.mp4"
        );
        bench.click("export.format");
        bench.click("export.format.webm");
        bench.harness_mut().run_steps(2);
        assert_eq!(bench.editor().preferences.export.format, ExportFormat::Webm);
        // Leaving the silent format re-enables the rows it had disabled; Video, ticked
        // alone, stays ticked.
        for id in [
            "export.codec",
            "export.bitrate",
            "export.profile",
            "export.audio.include",
            "export.gif.include",
        ] {
            assert!(enabled(&bench, id), "{id} applies to WebM");
        }
        assert!(!enabled(&bench, "export.video.include"));
        bench.click("export.scene");
        assert!(
            enabled(&bench, "export.scene.all"),
            "all scenes apply to WebM"
        );
        bench.click("export.scene");
        bench.click("export.close");
        open(&mut bench);
        assert_eq!(bench.editor().preferences.export.format, ExportFormat::Webm);
        assert!(matches!(
            bench.editor_mut().export_dialog().unwrap().start(
                Arc::new(project(true, false)),
                ExportOptions::default(),
                media::EncodePreference::default()
            ),
            Err(Error::EmptyTimeline)
        ));
    }
    // The Bit rate presets and their encoder settings; a Custom Kbps rate replaces the rate.
    for (quality, q, b) in [
        (ExportQuality::Lower, 70, 6_000_000),
        (ExportQuality::Recommended, 85, 12_000_000),
        (ExportQuality::Higher, 95, 24_000_000),
    ] {
        let settings = ExportOptions {
            quality,
            ..Default::default()
        }
        .settings(&project(false, false));
        assert_eq!((settings.quality, settings.bit_rate), (q, b));
        assert!(matches!(settings.mode, render::ExportMode::Encode));
    }
    let custom = ExportOptions {
        output: render::OutputOptions {
            bit_rate: Some(50_000_000),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut settings = custom.settings(&project(false, false));
    custom.output.apply(&mut settings).unwrap();
    assert_eq!(settings.bit_rate, 50_000_000);
}

#[test]
fn size_estimate_follows_the_encoder_settings_of_every_preset() {
    let project = project(false, false);
    let hd60 = |quality, include_audio| {
        let mut options = ExportOptions {
            quality,
            include_audio,
            ..Default::default()
        };
        options.output.resolution = Some([1920, 1080]);
        options.output.fps = Some(time::FrameRate::new(60, 1).unwrap());
        options
    };
    let mb =
        |options: &ExportOptions| super::settings::estimated_megabytes(options, &project, 9.58);
    let quality_mode = media::expected_rate_control(
        render::ExportSettings::from_project(&project).encoder,
        false,
    ) == media::RateControl::Quality;
    let default = mb(&hd60(ExportQuality::Recommended, true));
    let sizes: Vec<f64> = [
        ExportQuality::Lower,
        ExportQuality::Recommended,
        ExportQuality::Higher,
    ]
    .into_iter()
    .map(|quality| {
        let size = mb(&hd60(quality, true));
        let target = 9.58 * (quality.value().1 as f64 + 192_000.0) / 8_000_000.0;
        if quality_mode {
            // Quality-mode encoder (macOS VideoToolbox): never below the target rate.
            assert!(size >= target - 1e-9, "{size}");
        } else {
            // Bit-rate encoder (Windows): the target rate plus the audio track.
            assert!((size - target).abs() < 1e-9, "{size} {target}");
        }
        size
    })
    .collect();
    if quality_mode {
        // The measured export: 1080p60 at the default preset wrote 47.25 MB, not 14.6 MB.
        assert!((default - 47.25).abs() < 1.0, "{default}");
    } else {
        // Windows, 10 s 1080p30 at the defaults wrote 15.98 MB against the 12 Mb/s target.
        assert!((default - 9.58 * 12_192_000.0 / 8_000_000.0).abs() < 1e-9);
    }
    assert!(sizes.windows(2).all(|pair| pair[0] < pair[1]), "{sizes:?}");
    // Audio adds its track, resolution and frame rate scale the video, a custom rate is a floor.
    let silent = mb(&hd60(ExportQuality::Recommended, false));
    assert!((default - silent - 9.58 * 192_000.0 / 8_000_000.0).abs() < 1e-9);
    let mut small = hd60(ExportQuality::Recommended, true);
    small.output.resolution = Some([960, 540]);
    if quality_mode {
        assert!(mb(&small) < default / 3.0);
    } else {
        // A bit-rate encoder writes the target at any resolution.
        assert!((mb(&small) - default).abs() < 1e-9);
    }
    let mut custom = hd60(ExportQuality::Lower, true);
    custom.output.bit_rate = Some(50_000_000);
    assert!(mb(&custom) >= 9.58 * 50_192_000.0 / 8_000_000.0 - 1e-9);
    // Audio-only output estimates the audio track alone.
    let mut audio = ExportOptions {
        format: ExportFormat::M4a,
        ..Default::default()
    };
    audio.output.audio_only = true;
    assert!((mb(&audio) - 9.58 * 192_000.0 / 8_000_000.0).abs() < 1e-9);
    // GIF has its own model: the measured 3 s of camera footage at 240P (426x240, 29.97 fps)
    // wrote 6.05 MB, it is silent, and a smaller palette writes less.
    let mut gif = ExportOptions {
        format: ExportFormat::Gif,
        ..Default::default()
    };
    gif.output.resolution = Some([426, 240]);
    gif.output.fps = Some(time::FrameRate::new(30_000, 1_001).unwrap());
    let three = super::settings::estimated_megabytes(&gif, &project, 3.0);
    assert!((three - 6.05).abs() < 0.01, "{three}");
    gif.include_audio = true;
    assert_eq!(
        super::settings::estimated_megabytes(&gif, &project, 3.0),
        three
    );
    gif.output.gif.max_colors = 16;
    let small = super::settings::estimated_megabytes(&gif, &project, 3.0);
    assert!((small - three / 2.0).abs() < 0.01, "{small}");
}

#[test]
fn export_progress_then_save_writes_toast_and_closes_without_an_extra_screen() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("My Project.mp4");
    let mut bench = bench(editor(false, false));
    let project_before = bench.editor().session.project().clone();
    // The command must create the dialog with the shared preview/export render state.
    bench.editor_mut().activate("export.run").unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.finished.as_ref().is_some_and(|path| path.exists()));
    assert!(!state.open || state.phase == super::ExportPhase::Ready);
    for id in ["export.retry", "export.download", "export.copy_file"] {
        assert!(
            !bench
                .editor()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|w| w.id == id)
        );
    }
    assert!(enabled(&bench, "export.save"));
    assert!(Arc::ptr_eq(
        &project_before,
        bench.editor().session.project()
    ));
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut bench);
    assert!(destination.exists());
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(!state.open);
    assert!(state.finished.is_none());
    assert!(bench.editor_mut().activate("export.retry").is_err());
    let toast = bench
        .editor()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("My Project.mp4"), "{toast}");
}

#[test]
fn export_form_scrolls_the_caption_outputs_above_the_fixed_footer() {
    let mut bench = bench(editor(true, false));
    open(&mut bench);
    let before = rect(&bench, "export.captions.srt");
    bench.harness_mut().hover_at(egui::pos2(900.0, 400.0));
    for _ in 0..10 {
        bench.harness_mut().event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -100.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        bench.harness_mut().run_steps(2);
    }
    let after = rect(&bench, "export.captions.srt");
    let footer = rect(&bench, "export.run");
    assert!(
        after.top() < before.top(),
        "the form did not scroll: {before:?} {after:?}"
    );
    assert!(
        after.bottom() <= footer.top(),
        "the caption outputs scroll under the footer: {after:?} {footer:?}"
    );
}

fn rect(bench: &DialogBench, id: &str) -> egui::Rect {
    bench
        .editor()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .find(|w| w.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .rect
}

#[test]
fn export_footer_stays_fixed_below_the_form_at_every_window_height() {
    for height in [800.0, 600.0, 460.0] {
        let mut bench = bench(editor(false, false));
        bench.harness_mut().set_size(egui::vec2(1280.0, height));
        open(&mut bench);
        bench.harness_mut().run_steps(2);
        let footer = rect(&bench, "export.run");
        assert!(
            footer.bottom() <= height,
            "{height}: the footer button leaves the window: {footer:?}"
        );
        assert_eq!(footer.top(), rect(&bench, "export.close").top(), "{height}");
        // The first form row sits above the footer; the rest scroll inside the dialog.
        assert!(
            rect(&bench, "export.resolution").bottom() <= footer.top(),
            "{height}: the form starts under the footer"
        );
    }
}

#[test]
fn export_failure_panel_copies_only_diagnostics_and_retry_keeps_settings() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("retried.webm");
    let mut bench = bench(editor(false, true));
    open(&mut bench);
    let (sources, _) = restored_source(false);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.format");
    bench.click("export.format.webm");
    bench.click("export.bitrate");
    bench.click("export.bitrate.higher");
    // The video's own sound is not a dialog choice; the fixture silences it directly.
    bench.editor_mut().preferences.export.include_audio = false;
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    let state = bench.editor().export_state().unwrap().inspect();
    assert_eq!(
        state.attempt,
        Some(ExportOptions {
            format: ExportFormat::Webm,
            quality: ExportQuality::Higher,
            include_audio: false,
            ..Default::default()
        })
    );
    assert_eq!(state.unavailable.len(), 1);
    let diagnostics = state.diagnostics.clone().unwrap();
    // Settings may be edited while the failure panel is visible. Retry and its
    // eventual save dialog must still describe the completed attempt's container.
    bench.editor_mut().preferences.export.format = ExportFormat::Mp4;
    assert!(!diagnostics.is_empty());
    assert!(enabled(&bench, "export.copy"));
    assert!(enabled(&bench, "export.retry"));
    assert!(
        !bench
            .editor()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == "export.run")
    );
    bench
        .harness_mut()
        .get(egui_kittest::kittest::By::new().predicate(|n| n.author_id() == Some("export.copy")))
        .click();
    bench.harness_mut().run_steps(1);
    assert!(
        bench
            .harness()
            .output()
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(
                command,
                egui::OutputCommand::CopyText(text) if text == &diagnostics
            ))
    );
    let (sources, _) = restored_source(true);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.retry");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert_eq!(bench.editor().export_save_format(), ExportFormat::Webm);
    assert!(bench.editor().export_save_defaults().0.ends_with(".webm"));
    assert_eq!(
        bench.editor().export_state().unwrap().inspect().attempt,
        Some(ExportOptions {
            format: ExportFormat::Webm,
            quality: ExportQuality::Higher,
            include_audio: false,
            ..Default::default()
        })
    );
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    wait(&mut bench);
    assert!(destination.exists());
}

#[test]
fn cancelling_export_returns_to_settings_and_leaves_no_final_file() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("cancelled.mp4");
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let mut bench = bench(editor(false, true));
    let _release = Release(gate.clone());
    open(&mut bench);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(Arc::new(GateFactory(gate.clone())))
        .unwrap();
    bench.click("export.run");
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(bench.editor().export_state().unwrap().active());
    let running_job = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .job
        .unwrap()
        .id;
    assert!(bench.editor_mut().activate("export.retry").is_err());
    assert_eq!(
        bench
            .editor()
            .export_state()
            .unwrap()
            .inspect()
            .job
            .unwrap()
            .id,
        running_job
    );
    bench.click("export.cancel");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.open);
    assert!(state.finished.is_none());
    assert!(state.job.is_none());
    assert!(!destination.exists());
}

#[test]
fn missing_media_refusal_needs_consent_and_retry_reports_the_same_ranges() {
    for count in [1, 20] {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("degraded.mp4");
        let mut data = project(false, true).into_data();
        let first = data.scene().timeline.main.items[0].clone();
        for index in 1..count {
            let mut item = first.clone();
            item.id = project::ItemId(index + 1);
            item.start = Tick(first.source_range.duration().0 * index as i64);
            data.scene_mut().timeline.main.items.push(item);
        }
        let mut bench = bench(editor_project(Project::new(data).unwrap()));
        let project_before = bench.editor().session.project().clone();
        open(&mut bench);
        let (sources, _) = restored_source(false);
        bench
            .editor_mut()
            .export_dialog()
            .unwrap()
            .set_sources(sources)
            .unwrap();
        bench.click("export.run");
        assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
        let state = bench.editor().export_state().unwrap().inspect();
        assert_eq!(state.unavailable.len(), count as usize);
        assert_eq!(state.unavailable[0].asset, AssetId(1));
        assert_eq!(
            state.unavailable[0].range,
            SourceRange::new(Tick::ZERO, Tick(SECOND / 10)).unwrap()
        );
        assert!(!state.consent);
        assert!(Arc::ptr_eq(
            &project_before,
            bench.editor().session.project()
        ));
        let retry = bench
            .editor()
            .inspect()
            .unwrap()
            .widgets
            .into_iter()
            .find(|widget| widget.id == "export.retry")
            .unwrap();
        assert!(
            retry.rect.bottom() <= 800.0,
            "failure footer clipped: {:?}",
            retry.rect
        );
        bench
            .harness_mut()
            .get(
                egui_kittest::kittest::By::new()
                    .predicate(|node| node.author_id() == Some("export.consent")),
            )
            .click_accesskit();
        bench.harness_mut().run_steps(3);
        assert!(bench.editor().export_state().unwrap().inspect().consent);
        bench.click("export.retry");
        assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
        bench
            .editor_mut()
            .export_picked_file(destination.clone())
            .unwrap();
        wait(&mut bench);
        assert!(destination.exists());
        assert!(Arc::ptr_eq(
            &project_before,
            bench.editor().session.project()
        ));
    }
}

#[test]
fn cancelled_save_keeps_the_finished_export_for_a_later_save() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("second-save.mp4");
    let mut data = project(false, true).into_data();
    let mut secondary = data.scene().clone();
    secondary.id = project::SceneId(2);
    secondary.is_main = false;
    secondary.name = "Second".into();
    for track in secondary.timeline.iter_mut() {
        track.id = project::TrackId(track.id.0 + 10);
        for item in &mut track.items {
            item.id = project::ItemId(item.id.0 + 10);
        }
    }
    data.scenes.insert(secondary).unwrap();
    let mut e = editor_project(Project::new(data).unwrap());
    e.preferences.export.output.scene = render::SceneChoice::All;
    let mut bench = bench(e);
    open(&mut bench);
    let (sources, _) = restored_source(true);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    bench.editor_mut().export_save_cancelled();
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.open);
    assert!(state.finished.as_ref().is_some_and(|path| path.exists()));
    let finished = state.finished.unwrap();
    assert!(
        bench
            .editor()
            .export_save_defaults()
            .0
            .ends_with("-scene-0.mp4")
    );
    assert!(bench.editor_mut().activate("export.retry").is_err());
    // A failed save must retain the finished output and leave existing data intact.
    let blocked = dir.path().join("directory.mp4");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("keep"), b"previous").unwrap();
    bench
        .editor_mut()
        .export_picked_file(blocked.clone())
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    // Let the modal settle after progress changes to the diagnostics layout.
    bench.harness_mut().run_steps(3);
    assert!(enabled(&bench, "export.copy"));
    assert!(enabled(&bench, "export.retry"));
    let diagnostics = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .diagnostics
        .unwrap();
    bench
        .harness_mut()
        .get(egui_kittest::kittest::By::new().predicate(|n| n.author_id() == Some("export.copy")))
        .click();
    bench.harness_mut().run_steps(1);
    assert_eq!(bench.harness().output().platform_output.commands.iter().filter(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &diagnostics)).count(), 1);
    bench.click("export.retry");
    assert_eq!(
        bench
            .editor()
            .export_state()
            .unwrap()
            .inspect()
            .finished
            .as_ref(),
        Some(&finished)
    );
    assert_eq!(std::fs::read(blocked.join("keep")).unwrap(), b"previous");
    assert!(finished.exists());
    assert!(!destination.exists());
    std::fs::write(&destination, b"previous export").unwrap();
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    // The second scene follows beside the first, with no second picker.
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    assert!(destination.exists());
    assert_ne!(std::fs::read(&destination).unwrap(), b"previous export");
    assert!(!finished.exists());
    let second_destination = dir.path().join("second-save-scene-2.mp4");
    assert!(second_destination.is_file());
    assert!(!bench.editor().export_state().unwrap().inspect().open);
}

#[test]
fn retry_reuses_settings_into_new_output_and_preserves_a_prior_file() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("prior.mp4");
    std::fs::write(&destination, b"previous").unwrap();
    let mut bench = bench(editor(false, true));
    open(&mut bench);
    let (sources, _) = restored_source(false);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    bench.click("export.retry");
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    bench.click("export.close");
    open(&mut bench);
    assert_eq!(
        bench.editor().export_state().unwrap().phase(),
        super::ExportPhase::Settings
    );
    assert!(enabled(&bench, "export.run"));
}

#[test]
fn filenames_are_sanitized_and_export_preferences_roundtrip() {
    assert_eq!(
        super::default_file_name("My: Project?", "mp4"),
        "My_ Project_.mp4"
    );
    assert_eq!(
        super::default_file_name("D\u{1ef1} \u{00e1}n phim", "webm"),
        "D\u{1ef1} \u{00e1}n phim.webm"
    );
    assert_eq!(super::default_file_name("  spaced  ", "mp4"), "spaced.mp4");
    assert_eq!(super::default_file_name("CON", "mov"), "_CON.mov");
    assert_eq!(super::default_file_name("LPT9.txt", "mp4"), "_LPT9.txt.mp4");
    assert_eq!(
        super::default_file_name("   ", "mp4"),
        "Untitled Project.mp4"
    );
    let options = ExportOptions {
        format: ExportFormat::Webm,
        quality: ExportQuality::Higher,
        include_audio: false,
        ..Default::default()
    };
    assert!(!options.settings(&project(false, false)).include_audio);
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    let preferences = Preferences {
        export: options,
        export_directory: Some(dir.path().join("exports")),
        ..Default::default()
    };
    store.save(&preferences).unwrap();
    let loaded = store.load_with_warning().0;
    assert_eq!(loaded.export, options);
    assert_eq!(loaded.export_directory, preferences.export_directory);
}

#[test]
fn export_output_panel_persists_scene_range_refuses_empty_and_never_edits_project() {
    let mut bench = bench(editor(false, false));
    let rate = bench.editor().session.project().data().canvas.fps;
    bench
        .editor_mut()
        .dispatch(AppCommand::Seek(rate.tick_of(1).unwrap()))
        .unwrap();
    bench.editor_mut().activate("export.mark_in").unwrap();
    bench
        .editor_mut()
        .dispatch(AppCommand::Seek(rate.tick_of(2).unwrap()))
        .unwrap();
    bench.editor_mut().activate("export.mark_out").unwrap();
    assert_eq!(
        bench.editor().preferences.export.output.range,
        render::RangeChoice::InOut {
            start: rate.tick_of(1).unwrap(),
            end: rate.tick_of(2).unwrap()
        }
    );
    let before = project::encode(bench.editor().session.project(), false).unwrap();
    open(&mut bench);
    bench.click("export.resolution");
    bench.click("export.resolution.720");
    assert_eq!(
        bench.editor().preferences.export.output.resolution,
        Some([720, 720])
    );
    bench.click("export.custom.size");
    assert!(enabled(&bench, "export.width"));
    for id in [
        "export.resolution",
        "export.fps",
        "export.codec",
        "export.profile",
        "export.scene",
        "export.range",
    ] {
        assert!(enabled(&bench, id), "{id}");
    }
    bench.click("export.range");
    assert!(!enabled(&bench, "export.range.selected"));
    bench.click("export.range.full");
    bench
        .editor_mut()
        .select([project::ItemId(1)].into())
        .unwrap();
    bench.click("export.range");
    bench.click("export.range.selected");
    assert!(matches!(
        bench.editor().preferences.export.output.range,
        render::RangeChoice::Selected { .. }
    ));
    bench.click("export.scene");
    bench.click("export.scene.all");
    bench.click("export.range");
    bench.click("export.range.in_out");
    assert!(enabled(&bench, "export.range.in"));
    bench.editor_mut().preferences.export.output.range = render::RangeChoice::InOut {
        start: Tick::ZERO,
        end: Tick::ZERO,
    };
    bench.harness_mut().run_steps(3);
    assert!(!enabled(&bench, "export.run"));
    let invalid = bench.editor().preferences.export;
    assert!(
        bench
            .editor_mut()
            .export_dialog()
            .unwrap()
            .start(
                Arc::new(project(false, false)),
                invalid,
                media::EncodePreference::default()
            )
            .is_err()
    );
    let fps = bench.editor().session.project().data().canvas.fps;
    bench.editor_mut().preferences.export.output.range = render::RangeChoice::InOut {
        start: fps.tick_of(1).unwrap(),
        end: fps.tick_of(2).unwrap(),
    };
    bench.editor_mut().preferences.export.output.resolution = Some([64, 64]);
    bench.editor_mut().preferences.export.output.bit_rate = Some(100_000);
    bench.harness_mut().run_steps(3);
    assert!(enabled(&bench, "export.run"));
    let options = bench.editor().preferences.export;
    bench
        .editor_mut()
        .dispatch(AppCommand::Export(options))
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert_eq!(
        project::encode(bench.editor().session.project(), false).unwrap(),
        before
    );
    bench.editor_mut().activate("edit.undo").unwrap();
    bench.editor_mut().activate("edit.redo").unwrap();
    assert_eq!(
        project::encode(bench.editor().session.project(), false).unwrap(),
        before
    );
}

#[test]
fn save_refuses_every_source_alias_without_replacing_media() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("original.mp4");
    std::fs::write(&source_path, b"original source media").unwrap();
    let mut data = project(false, true).into_data();
    let mut asset: AssetData = data.assets.remove(0).into();
    asset.path = source_path.clone();
    asset.content_hash = ContentHash::from_bytes(b"original source media");
    data.assets.push(SourceAsset::try_from(asset).unwrap());
    let session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(768 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut bench = bench(Editor::new(session, playback, Preferences::default()));
    open(&mut bench);
    let (sources, _) = restored_source(true);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(sources)
        .unwrap();
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    let before = serde_json::to_vec(bench.editor().session.project().as_ref()).unwrap();
    let finished = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .finished
        .unwrap();
    // The alias kinds (path, dot path, hard link) are the render destination table's rows.
    bench
        .editor_mut()
        .export_picked_file(source_path.clone())
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.diagnostics.unwrap().contains("source"));
    assert_eq!(state.finished.as_ref(), Some(&finished));
    assert_eq!(
        serde_json::to_vec(bench.editor().session.project().as_ref()).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(&source_path).unwrap(),
        b"original source media",
        "saving an export replaced source media"
    );
}

#[test]
fn publication_keeps_ui_responsive_and_cancel_preserves_previous_file() {
    struct StorageGate(Arc<Gate>);
    impl project::atomic::AtomicFs for StorageGate {
        fn sync(&self, _: &std::fs::File) -> std::io::Result<()> {
            self.0.entered.lock().unwrap().send(()).unwrap();
            let mut release = self.0.release.0.lock().unwrap();
            while !*release {
                release = self.0.release.1.wait(release).unwrap();
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("prior.mp4");
    std::fs::write(&destination, b"previous").unwrap();
    let mut bench = bench(editor(false, false));
    open(&mut bench);
    bench.click("export.run");
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    let finished = bench
        .editor()
        .export_state()
        .unwrap()
        .inspect()
        .finished
        .unwrap();
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let _release = Release(gate.clone());
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_publication_fs(Arc::new(StorageGate(gate.clone())))
        .unwrap();
    bench
        .editor_mut()
        .export_picked_file(destination.clone())
        .unwrap();
    bench.harness_mut().run_steps(1);
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    bench.harness_mut().run_steps(3);
    assert!(enabled(&bench, "export.cancel"));
    bench.click("export.cancel");
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert!(finished.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn ticked_outputs_become_one_job_each_in_a_fixed_order() {
    let all = ExportOptions {
        format: ExportFormat::Webm,
        also: AlsoOutputs {
            audio: Some(ExportFormat::AudioWebm),
            gif: Some([320, 320]),
            captions: Some(captions::Format::Srt),
        },
        output: render::OutputOptions {
            resolution: Some([1280, 720]),
            fps: Some(time::FrameRate::FPS_24),
            bit_rate: Some(5_000_000),
            codec: render::VideoCodec::Vp9,
            range: render::RangeChoice::InOut {
                start: Tick::ZERO,
                end: Tick(SECOND / 20),
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let formats = |options: ExportOptions| -> Vec<ExportFormat> {
        options.outputs().iter().map(|o| o.format).collect()
    };
    let outputs = all.outputs();
    assert_eq!(
        formats(all),
        [
            ExportFormat::Webm,
            ExportFormat::AudioWebm,
            ExportFormat::Gif
        ]
    );
    // The primary output keeps every video choice; each further job is a single output.
    assert_eq!(outputs[0].output, all.output);
    assert!(outputs.iter().all(|o| o.also == AlsoOutputs::default()));
    // Audio carries no video field, GIF its own size and the video's frame rate; scene and
    // range are shared by all.
    let audio = outputs[1].output;
    assert_eq!(
        (audio.resolution, audio.fps, audio.bit_rate, audio.codec),
        (None, None, None, render::VideoCodec::Automatic)
    );
    let gif = outputs[2].output;
    assert_eq!(
        (gif.resolution, gif.fps, gif.bit_rate, gif.codec),
        (
            Some([320, 320]),
            Some(time::FrameRate::FPS_24),
            None,
            render::VideoCodec::Automatic
        )
    );
    assert!(
        outputs
            .iter()
            .all(|o| o.output.range == all.output.range && o.output.scene == all.output.scene)
    );
    // A PNG sequence has no sound, and a primary audio or GIF output adds no copy of itself.
    let sequence = ExportOptions {
        format: ExportFormat::PngSequence,
        ..all
    };
    assert_eq!(
        formats(sequence),
        [ExportFormat::PngSequence, ExportFormat::Gif]
    );
    let audio_first = ExportOptions {
        format: ExportFormat::M4a,
        ..all
    };
    assert_eq!(formats(audio_first), [ExportFormat::M4a, ExportFormat::Gif]);
    assert_eq!(audio_first.outputs()[1].output.fps, None);
    assert_eq!(ExportOptions::default().outputs().len(), 1);
}

#[test]
fn export_sections_tick_together_and_one_stays_ticked() {
    let mut bench = bench(editor(false, false));
    open(&mut bench);
    // Video alone is ticked and cannot be unticked.
    assert!(!enabled(&bench, "export.video.include"));
    assert!(enabled(&bench, "export.audio.include"));
    assert!(enabled(&bench, "export.gif.include"));
    assert!(!enabled(&bench, "export.audio.format"));
    // Captions join the group only when the project holds a caption item.
    assert!(!enabled(&bench, "export.captions.include"));
    assert!(!enabled(&bench, "export.captions.format"));
    bench.click("export.audio.include");
    bench.click("export.gif.include");
    bench.harness_mut().run_steps(2);
    let all = bench.editor().preferences.export;
    assert_eq!(all.format, ExportFormat::Mp4);
    assert_eq!(all.also.audio, Some(ExportFormat::M4a));
    assert_eq!(all.also.gif, Some([240, 240]));
    for id in [
        "export.video.include",
        "export.audio.include",
        "export.gif.include",
        "export.audio.format",
        "export.gif.resolution",
        "export.resolution",
        "export.run",
    ] {
        assert!(enabled(&bench, id), "{id} with all three ticked");
    }
    // The Audio Format and GIF Resolution rows edit the files beside the video.
    bench.click("export.audio.format");
    bench.click("export.audio.format.opus");
    bench.click("export.gif.resolution");
    bench.click("export.gif.resolution.320");
    let all = bench.editor().preferences.export;
    assert_eq!(all.also.audio, Some(ExportFormat::AudioWebm));
    assert_eq!(all.also.gif, Some([320, 320]));
    assert_eq!(all.output.resolution, None);
    // Unticking Video makes Audio the primary output and keeps the GIF.
    bench.click("export.video.include");
    bench.harness_mut().run_steps(2);
    let rest = bench.editor().preferences.export;
    assert_eq!(rest.format, ExportFormat::AudioWebm);
    assert_eq!(rest.also.audio, None);
    assert_eq!(rest.also.gif, Some([320, 320]));
    assert!(!enabled(&bench, "export.resolution"));
    // Unticking Audio leaves the GIF alone, which can no longer be unticked.
    bench.click("export.audio.include");
    bench.harness_mut().run_steps(2);
    let gif = bench.editor().preferences.export;
    assert_eq!(gif.format, ExportFormat::Gif);
    assert_eq!(gif.output.resolution, Some([320, 320]));
    assert_eq!(gif.also, AlsoOutputs::default());
    assert!(!enabled(&bench, "export.gif.include"));
}

#[test]
fn ticked_outputs_render_one_after_another_and_save_under_one_toast() {
    let audio = media::audio_export_container_available(ExportFormat::M4a.container());
    let gif = media::export_container_available(media::Container::Gif);
    let dir = tempfile::tempdir().unwrap();
    let mut bench = bench(editor(false, false));
    bench.editor_mut().preferences.export.also = AlsoOutputs {
        audio: audio.then_some(ExportFormat::M4a),
        gif: gif.then_some([240, 240]),
        ..Default::default()
    };
    let mut names = vec!["Export fixture.mp4"];
    if audio {
        names.push("Export fixture-audio.m4a");
    }
    if gif {
        names.push("Export fixture.gif");
    }
    bench.editor_mut().activate("export.run").unwrap();
    // One progress covers every job: the dialog is running, not ready, until all rendered.
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    assert_eq!(&bench.editor().export_save_defaults().0, names[0]);
    // One place is picked, for the first file; the others are written beside it.
    bench
        .editor_mut()
        .export_picked_file(dir.path().join("out.mp4"))
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    let names: Vec<&str> = names
        .iter()
        .map(|name| match *name {
            "Export fixture.mp4" => "out.mp4",
            "Export fixture-audio.m4a" => "out-audio.m4a",
            _ => "out.gif",
        })
        .collect();
    for name in &names {
        assert!(dir.path().join(name).is_file(), "{name} was not written");
    }
    if gif {
        let bytes = std::fs::read(dir.path().join("out.gif")).unwrap();
        assert!(bytes.starts_with(b"GIF8"));
    }
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(!state.open);
    assert!(state.finished.is_none());
    // One toast names every file.
    let toast = bench
        .editor()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    for name in &names {
        assert!(toast.contains(name), "{toast}");
    }
}

#[test]
fn cancelling_a_multi_output_export_starts_no_further_job() {
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let mut bench = bench(editor(false, true));
    let _release = Release(gate.clone());
    bench.editor_mut().preferences.export.also = AlsoOutputs {
        audio: Some(ExportFormat::M4a),
        gif: Some([240, 240]),
        ..Default::default()
    };
    open(&mut bench);
    bench
        .editor_mut()
        .export_dialog()
        .unwrap()
        .set_sources(Arc::new(GateFactory(gate.clone())))
        .unwrap();
    bench.editor_mut().activate("export.run").unwrap();
    bench.harness_mut().run_steps(2);
    recv.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(bench.editor().export_state().unwrap().active());
    assert!(enabled(&bench, "export.cancel"));
    bench.click("export.cancel");
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    for _ in 0..20 {
        bench.editor_mut().tick();
        bench.step();
    }
    let state = bench.editor().export_state().unwrap().inspect();
    assert!(state.job.is_none());
    assert!(state.finished.is_none());
    assert!(!bench.editor().export_state().unwrap().active());
}

#[test]
fn a_derived_name_that_exists_refuses_the_group_until_replace_is_ticked() {
    if !media::audio_export_container_available(ExportFormat::M4a.container()) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let taken = dir.path().join("out-audio.m4a");
    std::fs::write(&taken, b"previous").unwrap();
    let mut bench = bench(editor(false, false));
    bench.editor_mut().preferences.export.also = AlsoOutputs {
        audio: Some(ExportFormat::M4a),
        ..Default::default()
    };
    bench.editor_mut().activate("export.run").unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    bench.harness_mut().run_steps(2);
    assert!(enabled(&bench, "export.replace"));
    // Nothing is written and nothing replaced while a target of the group exists.
    bench
        .editor_mut()
        .export_picked_file(dir.path().join("out.mp4"))
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Failed);
    assert!(!dir.path().join("out.mp4").exists());
    assert_eq!(std::fs::read(&taken).unwrap(), b"previous");
    // One Replace choice covers the group.
    bench.harness_mut().run_steps(3);
    bench.click("export.replace");
    bench.click("export.retry");
    bench.harness_mut().run_steps(2);
    bench
        .editor_mut()
        .export_picked_file(dir.path().join("out.mp4"))
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    assert!(dir.path().join("out.mp4").is_file());
    assert_ne!(std::fs::read(&taken).unwrap(), b"previous");
}

/// A ticked Captions section writes its file beside the saved media even when the Captions
/// panel was never shown (the panel had no caption item picked and refused the write).
#[test]
fn ticked_captions_write_their_file_beside_the_media_without_opening_the_panel() {
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("cues.srt");
    let text = "1\n00:00:00,000 --> 00:00:00,100\nHello group\n";
    std::fs::write(&srt, text).unwrap();
    let imported = captions::parse(
        text.as_bytes(),
        captions::Format::Srt,
        captions::LanguageTag::parse("en").unwrap(),
    )
    .unwrap();
    let end = Tick(SECOND / 10);
    let mut session = Session::new(project(false, false), 10).unwrap();
    session
        .apply(engine::Command::AddAsset {
            asset: SourceAsset::try_from(AssetData {
                ephemeral: true,
                id: AssetId(2),
                path: std::fs::canonicalize(&srt).unwrap(),
                content_hash: ContentHash::from_bytes(text.as_bytes()),
                probe: media::SourceProbe {
                    format: "srt".into(),
                    duration: Some(end),
                    streams: Vec::new(),
                },
            })
            .unwrap(),
        })
        .unwrap();
    session
        .apply(engine::Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(2)),
                source_range: SourceRange::new(Tick::ZERO, end).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Caption(project::CaptionParams {
                    emphasis: Default::default(),
                    bubble: None,
                    effect: None,
                    word_highlight: false,
                    preset: None,
                    overrides: Default::default(),
                    cues: imported.track,
                }),
            },
        })
        .unwrap();
    let mut bench = bench(editor_project((**session.project()).clone()));
    bench.editor_mut().preferences.export.also = AlsoOutputs {
        captions: Some(captions::Format::Srt),
        ..Default::default()
    };
    bench.editor_mut().activate("export.run").unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Ready);
    bench
        .editor_mut()
        .export_picked_file(dir.path().join("out.mp4"))
        .unwrap();
    assert_eq!(wait(&mut bench), super::ExportPhase::Settings);
    // The caption file is a job of the Captions panel; it lands a few frames later.
    let caption = dir.path().join("out.srt");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !caption.is_file() {
        assert_eq!(bench.editor().error, None);
        assert!(
            Instant::now() < deadline,
            "the caption file was not written"
        );
        bench.editor_mut().tick();
        bench.step();
    }
    assert!(dir.path().join("out.mp4").is_file());
    let written = std::fs::read_to_string(caption).unwrap();
    assert!(written.contains("Hello group"), "{written}");
    // The caption job finishes after the group saved: its "Done" toast must not replace the
    // group toast that names every file.
    for _ in 0..30 {
        bench.editor_mut().tick();
        bench.step();
    }
    let toast = bench
        .editor()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("out.mp4"), "{toast}");
    assert!(!toast.contains("Done"), "{toast}");
}

#[test]
fn audio_formats_list_in_the_reference_order_and_map_to_their_containers() {
    let listed: Vec<_> = super::settings::AUDIO_FORMATS
        .iter()
        .map(|(format, _)| *format)
        .collect();
    // The reference's order is MP3, WAV, AAC, FLAC; Opus is ours, after them.
    assert_eq!(
        listed,
        [
            ExportFormat::Mp3,
            ExportFormat::Wav,
            ExportFormat::M4a,
            ExportFormat::Flac,
            ExportFormat::AudioWebm,
        ]
    );
    for (format, extension, container) in [
        (ExportFormat::Mp3, "mp3", media::Container::Mp3),
        (ExportFormat::Wav, "wav", media::Container::Wav),
        (ExportFormat::Flac, "flac", media::Container::Flac),
    ] {
        assert_eq!(format.extension(), extension);
        assert!(format.audio_only());
        assert!(!format.is_sequence() && !format.is_gif());
        assert!(format.unused_encoder_reason().is_some());
        let built = project(false, false);
        let settings = ExportOptions {
            format,
            ..Default::default()
        }
        .settings(&built);
        assert_eq!(settings.container, container);
        assert!(settings.audio_only);
    }
}

/// A persistent editor over the fixture project: its engine store can hold a cover.
fn persistent_editor(root: &std::path::Path) -> Editor {
    let project = project(false, false);
    let autosave = engine::recovery::Autosave::new(
        &root.join("recovery"),
        Session::new(project.clone(), 10).unwrap(),
        ByteBudget::new(64 << 20).unwrap(),
        engine::footage::uncorrected(),
        Arc::new(engine::jobs::SystemClock::default()),
        Default::default(),
    )
    .unwrap();
    let mut editor = editor_project(project);
    editor.session = crate::services::lifecycle::EditorSession::Persistent(Box::new(autosave));
    editor
}

#[test]
fn the_cover_waits_for_its_image_and_edit_cover_stores_the_playhead_frame() {
    let root = tempfile::tempdir().unwrap();
    let mut bench = bench(persistent_editor(root.path()));
    open(&mut bench);
    assert!(enabled(&bench, "export.cover.edit"));
    assert_eq!(bench.editor().session.metadata().cover_tick, Tick::ZERO);
    assert!(matches!(
        bench.editor().session.metadata().thumbnail,
        project::Thumbnail::Missing
    ));
    let history = bench.editor().session.history_len();
    bench.editor_mut().seek(Tick(SECOND / 30)).unwrap();
    let at = bench.editor().playback.position().unwrap();
    assert_ne!(at, Tick::ZERO);
    bench.click("export.cover.edit");
    let metadata = bench.editor().session.metadata();
    assert_eq!(metadata.cover_tick, at);
    assert!(matches!(
        metadata.thumbnail,
        project::Thumbnail::Ready { .. }
    ));
    // The cover is metadata beside the project, so no undo step is taken.
    assert_eq!(bench.editor().session.history_len(), history);
    assert_eq!(bench.editor().error, None);
    // The dialog stays open on its settings with the chip still live.
    assert!(bench.editor().export_state().unwrap().inspect().open);
    assert!(enabled(&bench, "export.cover.edit"));
}

#[test]
fn edit_cover_is_disabled_with_its_reason_when_nothing_can_store_a_cover() {
    let mut bench = bench(editor(false, false));
    open(&mut bench);
    assert!(!enabled(&bench, "export.cover.edit"));
    assert_eq!(
        bench.editor().session.metadata().thumbnail,
        project::Thumbnail::Missing
    );
}

#[test]
fn the_color_space_row_names_the_only_space_the_encoder_writes() {
    let mut bench = bench(editor(false, false));
    open(&mut bench);
    let harness = bench.harness();
    harness.get_by_label("Color space");
    harness.get_by_label("Rec. 709 SDR");
    // The row has no control: it is read-only text that carries the reason on hover.
    assert!(enabled(&bench, "export.color_space"));
}

/// A lent effect adapter that keeps every run on the CPU reference and counts its
/// frames. One frame acquisition at a time, like the device permit: a contended one is
/// refused (`Unsupported`), so render takes the CPU for it. A `held` adapter signals
/// its first acquisition on `entered` and parks it on a closed gate until `open`.
#[derive(Default)]
struct Lent {
    frames: AtomicUsize,
    refused: AtomicUsize,
    busy: AtomicBool,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    closed: Mutex<bool>,
    gate: Condvar,
}
impl Lent {
    fn open(&self) {
        *self.closed.lock().unwrap() = false;
        self.gate.notify_all();
    }
}
impl render::effects::EffectCompositor for Lent {
    fn frame<'a>(
        &'a self,
        hooks: &'a dyn render::ExecutionHooks,
    ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
        if self.busy.swap(true, Ordering::SeqCst) {
            self.refused.fetch_add(1, Ordering::SeqCst);
            return Err(render::Error::Unsupported("lent adapter busy"));
        }
        self.frames.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
        }
        drop(
            self.gate
                .wait_while(self.closed.lock().unwrap(), |closed| *closed)
                .unwrap(),
        );
        self.busy.store(false, Ordering::SeqCst);
        if hooks.cancelled() {
            return Err(render::Error::Cancelled);
        }
        Ok(None)
    }
    fn release_idle(&self) -> render::Result<()> {
        Ok(())
    }
    fn supports(&self, _: &str) -> bool {
        false
    }
    fn apply(
        &self,
        _: &render::Image,
        _: &[render::effects::EffectPass],
        _: render::effects::EffectInputs<'_>,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        Err(render::Error::Unsupported("lent adapter declines"))
    }
}
fn held() -> (Arc<Lent>, mpsc::Receiver<()>) {
    let (send, entered) = mpsc::channel();
    let lent = Lent {
        entered: Mutex::new(Some(send)),
        closed: Mutex::new(true),
        ..Lent::default()
    };
    (Arc::new(lent), entered)
}
fn frames_options() -> ExportOptions {
    ExportOptions {
        format: ExportFormat::PngSequence,
        include_audio: false,
        ..ExportOptions::default()
    }
}
fn lent_dialog(lent: Option<&Arc<Lent>>, budget: &ByteBudget) -> super::ExportDialog {
    let mut dialog = super::ExportDialog::new(budget.clone(), Arc::default()).unwrap();
    if let Some(lent) = lent {
        dialog.set_effects(lent.clone());
    }
    dialog
}
fn start(dialog: &mut super::ExportDialog, project: &Arc<Project>) {
    dialog
        .start(project.clone(), frames_options(), Default::default())
        .unwrap();
}
/// Start `dialog`'s attempt and return once its first frame waits on the adapter's gate.
fn start_held(
    dialog: &mut super::ExportDialog,
    project: &Arc<Project>,
    entered: &mpsc::Receiver<()>,
) {
    start(dialog, project);
    // The runner starts a queued job on its next pump.
    dialog.tick(project, frames_options()).unwrap();
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
}
/// Tick every dialog until none runs.
fn tick_all(dialogs: &mut [&mut super::ExportDialog], project: &Arc<Project>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while dialogs.iter().any(|dialog| dialog.active()) {
        for dialog in dialogs.iter_mut() {
            dialog.tick(project, frames_options()).unwrap();
        }
        assert!(Instant::now() < deadline, "export stalled");
        std::thread::yield_now();
    }
}
/// The rendered PNG frames in frame order, without the staged directory's name.
fn frames_of(dialog: &super::ExportDialog) -> Vec<Vec<u8>> {
    let dir = dialog.inspect().finished.unwrap();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files.iter().map(|path| std::fs::read(path).unwrap()).collect()
}

/// Fails if export keeps `effects=None` after the adapter is lent, or if lending an
/// adapter that declines every run changes the frames the CPU-only export writes.
#[test]
fn lent_adapter_renders_every_export_frame_and_matches_the_cpu_reference() {
    let project = Arc::new(project(false, false));
    let budget = ByteBudget::new(768 << 20).unwrap();
    let lent = Arc::new(Lent::default());
    let mut cpu = lent_dialog(None, &budget);
    let mut gpu = lent_dialog(Some(&lent), &budget);
    start(&mut cpu, &project);
    tick_all(&mut [&mut cpu], &project);
    start(&mut gpu, &project);
    tick_all(&mut [&mut gpu], &project);
    assert_eq!(cpu.phase(), super::ExportPhase::Ready);
    assert_eq!(gpu.phase(), super::ExportPhase::Ready);
    let reference = frames_of(&cpu);
    assert!(!reference.is_empty());
    assert!(lent.frames.load(Ordering::SeqCst) >= reference.len());
    assert_eq!(frames_of(&gpu), reference);
}

/// Fails if a second export owner contending for the one shared adapter fails, or
/// writes other frames, instead of rendering the refused frame on the CPU.
#[test]
fn two_export_owners_share_one_adapter_and_a_refused_frame_renders_on_the_cpu() {
    let project = Arc::new(project(false, false));
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (lent, entered) = held();
    let mut first = lent_dialog(Some(&lent), &budget);
    let mut second = lent_dialog(Some(&lent), &budget);
    start_held(&mut first, &project, &entered);
    // The first owner holds the adapter's only permit for the whole second export.
    start(&mut second, &project);
    tick_all(&mut [&mut second], &project);
    lent.open();
    tick_all(&mut [&mut first], &project);
    assert_eq!(first.phase(), super::ExportPhase::Ready);
    assert_eq!(second.phase(), super::ExportPhase::Ready);
    assert!(lent.refused.load(Ordering::SeqCst) >= 1);
    assert_eq!(frames_of(&first), frames_of(&second));
}

/// Fails if Retry lends the adapter attached after the failed attempt rather than
/// the attempt's own, or cannot publish its new render to a fresh destination.
#[test]
fn retry_renders_with_the_attempt_adapter_into_a_fresh_destination() {
    let project = Arc::new(project(false, true));
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (attempt, later) = (Arc::new(Lent::default()), Arc::new(Lent::default()));
    let restored = Arc::new(AtomicBool::new(false));
    let mut dialog = lent_dialog(Some(&attempt), &budget);
    dialog
        .set_sources(Arc::new(Restorable {
            restored: restored.clone(),
            decoded: Arc::default(),
        }))
        .unwrap();
    start(&mut dialog, &project);
    tick_all(&mut [&mut dialog], &project);
    assert_eq!(dialog.phase(), super::ExportPhase::Failed);
    dialog.set_effects(later.clone());
    restored.store(true, Ordering::SeqCst);
    dialog.retry(project.clone()).unwrap();
    tick_all(&mut [&mut dialog], &project);
    assert_eq!(dialog.phase(), super::ExportPhase::Ready);
    assert!(attempt.frames.load(Ordering::SeqCst) > 0);
    assert_eq!(later.frames.load(Ordering::SeqCst), 0);
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("fresh");
    dialog.save_to(destination.clone(), Vec::new()).unwrap();
    tick_all(&mut [&mut dialog], &project);
    assert!(std::fs::read_dir(&destination).unwrap().count() > 0);
}

/// Fails if closing the dialog leaves the running export worker holding the lent
/// adapter, or its bytes, after the app may drop the device.
#[test]
fn closing_a_lent_export_cancels_and_joins_before_releasing_the_adapter() {
    let project = Arc::new(project(false, false));
    let budget = ByteBudget::new(768 << 20).unwrap();
    let (lent, entered) = held();
    let mut dialog = lent_dialog(Some(&lent), &budget);
    start_held(&mut dialog, &project, &entered);
    // The close runs while the worker waits on the gate; the gate opens beside it.
    std::thread::scope(|scope| {
        scope.spawn(|| lent.open());
        drop(dialog);
    });
    assert_eq!(Arc::strong_count(&lent), 1);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

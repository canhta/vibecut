#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use vibecut::{
    Editor, Error, Result,
    capture::Capture,
    catalog::{PreferenceStore, Preferences},
    egui,
    playback::{AudioPort, Files, Playback},
};

fn audio(port: Arc<AudioPort>, rate: u32) -> Result<cpal::Stream> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(Error::PreviewUnavailable("no output device"))?;
    let config = device
        .supported_output_configs()?
        .filter(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
        .find_map(|c| c.try_with_sample_rate(rate))
        .ok_or(Error::Unsupported(
            "device needs stereo f32 at the project sample rate",
        ))?;
    let failure = port.clone();
    let stream = device.build_output_stream(
        config.config(),
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let timestamp = info.timestamp();
            let latency = timestamp.playback.duration_since(timestamp.callback);
            let frames = (latency.as_nanos().saturating_mul(u128::from(rate)) / 1_000_000_000)
                .min(u128::from(u64::MAX)) as u64;
            port.fill(data, frames);
        },
        move |error| {
            if !matches!(
                error.kind(),
                cpal::ErrorKind::Xrun
                    | cpal::ErrorKind::RealtimeDenied
                    | cpal::ErrorKind::DeviceChanged
            ) {
                failure.fail();
            }
        },
        Some(Duration::from_secs(2)),
    )?;
    stream.play()?;
    Ok(stream)
}
struct Native {
    editor: Editor,
    store: PreferenceStore,
    _stream: Option<cpal::Stream>,
    _writer: Option<project::WriterLease>,
    smoke: Option<Smoke>,
    capture: Option<Capture>,
}
struct Smoke {
    started: Instant,
    seconds: u64,
    output: PathBuf,
    samples: Vec<serde_json::Value>,
    last_sample: u64,
    saved: bool,
    close_request_at: Option<u64>,
}
impl eframe::App for Native {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.editor.tick();
        ctx.request_repaint_after(Duration::from_millis(8));
        if let Some(smoke) = &mut self.smoke {
            let elapsed = smoke.started.elapsed().as_secs();
            if ctx.input(|input| input.viewport().close_requested()) {
                smoke.close_request_at = Some(elapsed);
            }
            if elapsed != smoke.last_sample {
                smoke.last_sample = elapsed;
                let position = self.editor.playback.position().unwrap_or(time::Tick::ZERO);
                let frame = self.editor.presented_frame();
                let budget = self.editor.playback.budget().snapshot().ok();
                smoke.samples.push(serde_json::json!({"elapsed_seconds":elapsed,"audio_tick":position.0,"video_tick":frame.map(|f|f.0),"video_lag_ticks":frame.map(|f|position.0-f.0),"budget_used":budget.as_ref().map(|b|b.used),"budget_limit":budget.as_ref().map(|b|b.limit),"underrun_frames":self.editor.playback.audio_port().underrun_frames(),"playing":self.editor.playback.playing(),"audio_device_failed":self.editor.playback.audio_port().failed(),"occluded":ctx.input(|input|input.viewport().occluded),"error":self.editor.playback.error().map(|e| format!("{e:?}"))}));
            }
            if elapsed >= 5 && !smoke.saved && self.capture.is_none() {
                match Capture::request(ctx, None, Duration::from_secs(3)) {
                    Ok(capture) => self.capture = Some(capture),
                    Err(e) => eprintln!("capture: {e}"),
                }
            }
            if let Some(capture) = &self.capture {
                match capture.poll() {
                    Ok(Some(image)) => {
                        let bytes: Vec<u8> =
                            image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        if let Err(e) = image::save_buffer(
                            smoke.output.join("window.png"),
                            &bytes,
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        ) {
                            eprintln!("capture save: {e}");
                        }
                        smoke.saved = true;
                        self.capture = None;
                    }
                    Err(e) => {
                        eprintln!("capture: {e}");
                        smoke.saved = true;
                        self.capture = None;
                    }
                    Ok(None) => {}
                }
            }
            if elapsed >= smoke.seconds {
                if let Ok(bytes) = serde_json::to_vec_pretty(&smoke.samples)
                    && let Err(e) = std::fs::write(smoke.output.join("playback.json"), bytes)
                {
                    eprintln!("evidence write: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.editor.ui(ui);
    }
    fn on_exit(&mut self) {
        if let Some(smoke) = &self.smoke {
            let elapsed = smoke.started.elapsed().as_secs_f64();
            let receipt = serde_json::json!({"elapsed_seconds":elapsed,"requested_seconds":smoke.seconds,"completed":elapsed >= smoke.seconds as f64,"window_close_request_at":smoke.close_request_at,"close_request_origin":"unavailable in eframe window event"});
            for (name, value) in [
                (
                    "playback.json",
                    serde_json::Value::Array(smoke.samples.clone()),
                ),
                ("exit.json", receipt),
            ] {
                if let Ok(bytes) = serde_json::to_vec_pretty(&value)
                    && let Err(error) = std::fs::write(smoke.output.join(name), bytes)
                {
                    eprintln!("observation receipt: {error}");
                }
            }
        }
        if let Err(e) = self.store.save(&self.editor.preferences) {
            eprintln!("preferences: {e}");
        }
    }
}
fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut file = None;
    let mut smoke = None;
    while let Some(arg) = args.next() {
        if arg == "--observe" {
            let seconds = args
                .next()
                .and_then(|s| s.to_str().and_then(|s| s.parse::<u64>().ok()))
                .filter(|s| *s > 0 && *s <= 3600)
                .ok_or(Error::Invalid("observation seconds"))?;
            let output = args
                .next()
                .map(PathBuf::from)
                .ok_or(Error::Invalid("observation output"))?;
            std::fs::create_dir_all(&output)?;
            smoke = Some(Smoke {
                started: Instant::now(),
                seconds,
                output,
                samples: Vec::with_capacity(seconds as usize),
                last_sample: u64::MAX,
                saved: false,
                close_request_at: None,
            });
        } else if file.replace(PathBuf::from(arg)).is_some() {
            return Err(Error::Invalid("one project path expected"));
        }
    }
    let (project, writer) = if let Some(path) = file {
        let lease = project::WriterLease::acquire(&path)?;
        let loaded = lease.load()?;
        (loaded.project, Some(lease))
    } else {
        (
            project::Project::empty(project::ProjectId(1), "Untitled project".into())?,
            None,
        )
    };
    let rate = project.data().settings.audio_sample_rate;
    let session = engine::Session::new(project, 100)?;
    let budget = engine::ByteBudget::new(768 << 20)?;
    let mut playback = Playback::new(&session, budget, Arc::new(Files))?;
    let stream = match audio(playback.audio_port(), rate) {
        Ok(stream) => {
            playback.set_device_available(true);
            Some(stream)
        }
        Err(e) => {
            eprintln!("audio unavailable: {e}");
            None
        }
    };
    if smoke.is_some() && session.project().duration() > time::Tick::ZERO {
        playback.seek(time::Tick::ZERO, true, session.project().clone())?;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("APPDATA"))
        .ok_or(Error::Unsupported("preference directory"))?;
    let store = PreferenceStore::new(PathBuf::from(home).join(".config/vibecut/preferences.json"));
    let preferences = match store.load() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("preferences unavailable: {e}");
            Preferences::default()
        }
    };
    let editor = Editor::new(session, playback, preferences);
    #[cfg(target_os = "macos")]
    let icon_bytes = include_bytes!("../../../assets/app-icon/vibecut-macos-512.png").as_slice();
    #[cfg(not(target_os = "macos"))]
    let icon_bytes = include_bytes!("../../../assets/app-icon/vibecut-1024.png").as_slice();
    let icon = image::load_from_memory(icon_bytes)?.into_rgba8();
    let icon = egui::IconData {
        width: icon.width(),
        height: icon.height(),
        rgba: icon.into_raw(),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1280.0, 800.0])
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native(
        "VibeCut",
        options,
        Box::new(move |_| {
            Ok(Box::new(Native {
                editor,
                store,
                _stream: stream,
                _writer: writer,
                smoke,
                capture: None,
            }))
        }),
    )
    .map_err(|_| Error::Unsupported("native window or GPU"))?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("VibeCut: {error}");
        std::process::exit(1);
    }
}

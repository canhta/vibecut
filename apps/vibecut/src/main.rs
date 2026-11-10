#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use vibecut::{
    Error, Result,
    capture::Capture,
    egui,
    i18n::{Locale, Text},
    preferences::{PreferenceStore, Preferences},
    services::playback::AudioPort,
    workspace::Workspace,
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
    control: Option<vibecut_cli::mcp::Server>,
    control_options: Option<(PathBuf, vibecut_cli::mcp::Grants)>,
    workspace: Workspace,
    effects: Option<Arc<vibecut::effects::GpuEffects>>,
    store: PreferenceStore,
    saved_preferences: Preferences,
    _stream: Option<cpal::Stream>,
    audio_port: Option<Arc<AudioPort>>,
    project_audio: BTreeMap<project::ProjectId, (Arc<AudioPort>, Option<cpal::Stream>)>,
    smoke: Option<Smoke>,
    capture: Option<Capture>,
    observe_create: bool,
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
        ctx.request_repaint_after(Duration::from_millis(8));
        self.workspace.tick();
        // Immediate child viewports are declared by the root UI pass. Keep
        // running that pass after Home is hidden, including the final close.
        if !self.workspace.home_open() {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::RequestPaintWhileHidden,
            );
        }
        if self.workspace.ready_to_close() {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        }
        if self.workspace.focused_project().is_some()
            && let Some((endpoint, grants)) = self.control_options.take()
        {
            match vibecut_cli::mcp::Server::bind_with_media(
                &endpoint,
                grants,
                self.workspace.control_budget(),
                self.workspace.media(),
            ) {
                Ok(server) => self.control = Some(server),
                Err(_) => tracing::warn!(
                    category = "control_listener_failed",
                    "Control listener unavailable"
                ),
            }
        }

        if let Some(control) = &mut self.control
            && vibecut::control::poll_workspace(&mut self.workspace, &mut control.inbox).is_err()
        {
            tracing::warn!(category = "control_failed", "Control tick failed");
        }
        if self.observe_create && !self.workspace.busy() && self.workspace.error().is_none() {
            self.observe_create = false;
            if let Err(e) = self.workspace.create() {
                tracing::warn!(category="observation_create_failed",error=%e,"Observation project unavailable");
            }
        }
        self.project_audio
            .retain(|id, _| self.workspace.windows.contains_key(id));
        for (id, window) in &mut self.workspace.windows {
            let Some(editor) = &mut window.editor else {
                continue;
            };
            if self.project_audio.contains_key(id) {
                continue;
            }
            if let Some(effects) = &self.effects
                && let Err(error) = editor.set_effects(effects.clone())
            {
                tracing::warn!(category = "effects_unavailable", error = %error, "GPU effects unavailable");
            }
            let port = editor.playback.audio_port();
            let rate = editor.session.project().data().settings.audio_sample_rate;
            let stream = match audio(port.clone(), rate) {
                Ok(stream) => {
                    editor.playback.set_device_available(true);
                    Some(stream)
                }
                Err(error) => {
                    tracing::warn!(category="audio_unavailable",error=%error,"Audio output unavailable");
                    None
                }
            };
            self.project_audio.insert(*id, (port, stream));
            if self.smoke.is_some() && editor.session.project().duration() > time::Tick::ZERO {
                let _ = editor.activate("transport.play");
            }
        }
        // Shorts retains its own preview device, independently of project audio.
        if let Some(shorts) = &mut self.workspace.shorts {
            let rate = shorts
                .preview_session()
                .map(|session| session.project().data().settings.audio_sample_rate);
            if let (Some(rate), Some(playback)) = (rate, shorts.preview_mut()) {
                let port = playback.audio_port();
                if self
                    .audio_port
                    .as_ref()
                    .is_none_or(|old| !Arc::ptr_eq(old, &port))
                {
                    if let Some(effects) = &self.effects {
                        let _ = playback.set_effects(effects.clone());
                    }
                    self._stream = audio(port.clone(), rate).ok();
                    playback.set_device_available(self._stream.is_some());
                    self.audio_port = Some(port);
                }
            }
        }
        let Some(editor) = self.workspace.focused_editor() else {
            return;
        };
        if let Some(smoke) = &mut self.smoke {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            let elapsed = smoke.started.elapsed().as_secs();
            if ctx.input(|input| input.viewport().close_requested()) {
                smoke.close_request_at = Some(elapsed);
            }
            if elapsed != smoke.last_sample {
                smoke.last_sample = elapsed;
                let position = editor.playback.position().unwrap_or(time::Tick::ZERO);
                let frame = editor.presented_frame();
                let budget = editor.playback.budget().snapshot().ok();
                smoke.samples.push(serde_json::json!({"elapsed_seconds":elapsed,"audio_tick":position.0,"video_tick":frame.map(|f|f.0),"decoded_video_tick":editor.playback.frame().map(|f|f.at.0),"video_lag_ticks":frame.map(|f|position.0-f.0),"budget_used":budget.as_ref().map(|b|b.used),"budget_limit":budget.as_ref().map(|b|b.limit),"underrun_frames":editor.playback.audio_port().underrun_frames(),"playing":editor.playback.playing(),"audio_device_failed":editor.playback.audio_port().failed(),"occluded":ctx.input(|input|input.viewport().occluded),"error":editor.playback.error().map(|e| format!("{e:?}"))}));
            }
            if elapsed >= 5 && !smoke.saved && self.capture.is_none() {
                match Capture::request(ctx, None, Duration::from_secs(3)) {
                    Ok(capture) => self.capture = Some(capture),
                    Err(e) => {
                        tracing::warn!(category = "capture_unavailable", error = %e, "Capture unavailable")
                    }
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
                            tracing::warn!(category = "capture_save_failed", kind = ?e, "Capture save failed");
                        }
                        smoke.saved = true;
                        self.capture = None;
                    }
                    Err(e) => {
                        tracing::warn!(category = "capture_unavailable", error = %e, "Capture unavailable");
                        smoke.saved = true;
                        self.capture = None;
                    }
                    Ok(None) => {}
                }
            }
            if elapsed >= smoke.seconds {
                self.workspace.request_close();
                if let Ok(bytes) = serde_json::to_vec_pretty(&smoke.samples)
                    && let Err(e) = std::fs::write(smoke.output.join("playback.json"), bytes)
                {
                    tracing::warn!(category = "evidence_write_failed", kind = ?e.kind(), "Evidence write failed");
                }
                if !self.workspace.busy()
                    && let Err(e) = self.workspace.persist(None, true, true, true)
                {
                    tracing::warn!(category="observation_flush_failed",error=%e,"Observation exit refused");
                }
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.workspace.ui(ui);
        // Persist a completed preference edit once, not on every drag frame.
        if self.workspace.preferences != self.saved_preferences
            && !ui.input(|input| input.pointer.any_down())
        {
            match self.store.save(&self.workspace.preferences) {
                Ok(()) => self.saved_preferences = self.workspace.preferences.clone(),
                Err(error) => {
                    // Retry on the next edit instead of logging on every frame.
                    self.saved_preferences = self.workspace.preferences.clone();
                    tracing::warn!(category = "preference_save_failed", error = %error, "Preferences could not be saved")
                }
            }
        }
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
                    tracing::warn!(category = "receipt_write_failed", kind = ?error.kind(), "Observation receipt write failed");
                }
            }
        }
        if let Err(e) = self.store.save(&self.workspace.preferences) {
            tracing::warn!(category = "preference_save_failed", error = %e, "Preferences could not be saved");
        }
    }
}
// Distribution startup check: the real workspace/frontend, without OS devices.
fn headless_smoke(root: PathBuf) -> Result<()> {
    if root.exists() {
        return Err(Error::Invalid("headless smoke needs a new directory"));
    }
    let mut workspace = Workspace::new(root, Preferences::default(), None)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut creating = false;
    loop {
        workspace.tick();
        if workspace.error().is_some() || Instant::now() >= deadline {
            return Err(Error::Invalid("headless workspace startup failed"));
        }
        if !workspace.busy() {
            if creating {
                break;
            }
            workspace.create()?;
            creating = true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if workspace.focused_editor().is_none() {
        return Err(Error::Invalid("headless editor missing"));
    }
    let context = egui::Context::default();
    for _ in 0..3 {
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| workspace.ui(ui),
        );
        let drew_shapes = !output.shapes.is_empty();
        output.drop_without_applying_deltas();
        if !drew_shapes {
            return Err(Error::Invalid("headless frontend produced no shapes"));
        }
    }
    Ok(())
}
fn run() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    // The actual app composition, without creating an OS window or audio device.
    // Background attach tests cannot activate a window that never exists.
    if arguments
        .first()
        .is_some_and(|arg| arg == "--control-background")
    {
        return vibecut::control::background(arguments.into_iter().skip(1).collect());
    }
    if arguments
        .first()
        .is_some_and(|arg| arg == "--headless-smoke")
    {
        if arguments.len() != 2 {
            return Err(Error::Invalid("headless smoke directory expected"));
        }
        let root = arguments.get(1).ok_or(Error::Invalid("smoke directory"))?;
        return headless_smoke(PathBuf::from(root));
    }
    let mut args = arguments.into_iter();
    let mut file = None;
    let mut smoke = None;
    let mut control_socket = None;
    let mut control_output = None;
    while let Some(arg) = args.next() {
        if arg == "--control-socket" {
            control_socket = Some(PathBuf::from(
                args.next().ok_or(Error::Invalid("control socket"))?,
            ));
        } else if arg == "--control-output-dir" {
            control_output = Some(PathBuf::from(
                args.next()
                    .ok_or(Error::Invalid("control output directory"))?,
            ));
        } else if arg == "--observe" {
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
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("APPDATA"))
        .ok_or(Error::Unsupported("preference directory"))?;
    let store = PreferenceStore::new(PathBuf::from(home).join(".config/vibecut/preferences.json"));
    let (preferences, warning) = store.load_with_warning();
    if let Some(issue) = warning {
        tracing::warn!(category = "preference_load_failed", %issue, "Preferences restored defaults");
    }
    let observe_create = smoke.is_some() && file.is_none();
    let mut workspace = Workspace::new(store.path().with_file_name("recovery"), preferences, file)?;
    if warning.is_some() {
        workspace.warn_preferences_reset();
    }
    let saved_preferences = workspace.preferences.clone();
    let control_options = control_socket
        .map(|endpoint| {
            let grants = match control_output.as_deref() {
                Some(path) => vibecut_cli::mcp::Grants::output(path)
                    .map_err(|_| Error::Invalid("control output directory"))?,
                None => vibecut_cli::mcp::Grants::default(),
            };
            Ok::<_, Error>((endpoint, grants))
        })
        .transpose()?;
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
        viewport: workspace
            .preferences
            .windows
            .get("home")
            .cloned()
            .unwrap_or(vibecut::preferences::WindowGeometry {
                size: vibecut::home::WINDOW,
                ..Default::default()
            })
            .builder()
            .with_icon(Arc::new(icon))
            .with_window_level(if smoke.is_some() {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }),
        ..Default::default()
    };
    eframe::run_native(
        vibecut::i18n::text(Locale::En, Text::AppName),
        options,
        Box::new(move |cc| {
            let effects = cc.wgpu_render_state.as_ref().and_then(|state| {
                match pollster::block_on(vibecut::effects::GpuEffects::new(state.device.clone(), state.queue.clone())) {
                    Ok(adapter) => Some(Arc::new(adapter)),
                    Err(error) => { tracing::warn!(category = "effects_unavailable", error = %error, "CPU effects fallback"); None }
                }
            });
            Ok(Box::new(Native {
                control: None,
                control_options,
                workspace,
                effects,
                store,
                saved_preferences,
                _stream: None,
                audio_port: None,
                project_audio: BTreeMap::new(),
                smoke,
                capture: None,
                observe_create,
            }))
        }),
    )?;
    Ok(())
}
fn main() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .try_init();
    if let Err(error) = run() {
        tracing::error!(category = "startup_failed", error = %error, "VibeCut could not start");
        std::process::exit(1);
    }
}

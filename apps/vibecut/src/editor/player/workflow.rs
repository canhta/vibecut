//! Applies player transport requests.
use crate::editor::{
    DEFAULT_ZOOM, Editor, Error, MAX_ZOOM, MIN_ZOOM_FLOOR, PathBuf, Result, Text, Tick, Vec2,
    display_time, drop_frame, egui, preview,
};

impl Editor {
    /// Restart preview/audio work against the current immutable project snapshot.
    /// Old generations are fenced by `Playback::seek`; the position is clamped when
    /// an edit shortens the timeline and the transport state is preserved when able.
    pub(in crate::editor) fn sync_preview(&mut self) {
        let project = self.session.project().clone();
        let duration = project.duration();
        let at = self
            .playback
            .position()
            .unwrap_or(Tick::ZERO)
            .clamp(Tick::ZERO, duration);
        let playing = self.playback.playing();
        if self.playback.seek(at, playing, project.clone()).is_err()
            && let Err(error) = self.playback.seek(at, false, project)
        {
            self.error = Some(error.to_string());
        }
    }
    pub(in crate::editor) fn preview_canvas_size(&self) -> [u32; 2] {
        let canvas = &self.session.project().data().canvas;
        [canvas.width, canvas.height]
    }
    pub(in crate::editor) fn format_timecode(&self, at: Tick) -> String {
        let rate = self.session.project().data().canvas.fps;
        time::format_timecode(at, rate, drop_frame(rate))
            .unwrap_or_else(|_| display_time(at.max(Tick::ZERO)))
    }
    pub(in crate::editor) fn current_timecode(&self) -> String {
        self.format_timecode(self.playback.position().unwrap_or(Tick::ZERO))
    }
    pub(in crate::editor) fn commit_timecode(&mut self) {
        let Some(edit) = self.timecode_edit.take() else {
            return;
        };
        let duration = self.session.project().duration();
        let rate = self.session.project().data().canvas.fps;
        match time::parse_timecode(&edit.text, rate, drop_frame(rate)) {
            Ok(at) if at >= Tick::ZERO && at <= duration => {
                if let Err(error) =
                    self.dispatch("playback.seek", serde_json::json!({"time": at.0}))
                {
                    self.error = Some(error.to_string());
                } else {
                    self.timecode_error = None;
                }
            }
            _ => self.timecode_error = Some(Text::TimecodeInvalid),
        }
    }
    pub(in crate::editor) fn cancel_timecode(&mut self) {
        self.timecode_edit = None;
        self.timecode_error = None;
    }
    pub(in crate::editor) fn set_preview_fit(&mut self) -> Result<()> {
        self.preview_view.set_fit();
        self.preferences
            .set("preview_fit", serde_json::Value::Bool(true))?;
        Ok(())
    }
    pub(in crate::editor) fn set_preview_zoom(
        &mut self,
        zoom: f32,
        viewport: Vec2,
        canvas: [u32; 2],
    ) -> Result<()> {
        let zoom = self
            .preview_view
            .set_zoom(zoom, viewport, canvas)
            .ok_or(Error::Invalid("preview zoom"))?;
        self.preferences
            .set("preview_zoom", serde_json::json!(zoom))?;
        self.preferences
            .set("preview_fit", serde_json::Value::Bool(false))?;
        Ok(())
    }
    pub(in crate::editor) fn preview_zoom_by(&mut self, factor: f32) -> Result<()> {
        let viewport = self.preview_viewport;
        let canvas = self.preview_canvas_size();
        let base = if self.preview_view.fit {
            self.preview_view.scale(viewport, canvas)
        } else {
            self.preview_view.zoom
        };
        let target = preview::step_zoom(base, factor);
        if (target - self.preview_view.zoom).abs() < f32::EPSILON && !self.preview_view.fit {
            return Ok(());
        }
        self.set_preview_zoom(target, viewport, canvas)
    }
    pub(in crate::editor) fn open_snapshot_dialog(&mut self) -> Result<()> {
        if self.session.project().duration() == Tick::ZERO {
            self.snapshot.error = Some(engine::control::ErrorCode::EmptyProject);
            return Err(Error::Engine(engine::EditError::EmptyProject));
        }
        self.snapshot.open();
        self.lifecycle_action = Some(crate::lifecycle::EditorAction::SnapshotSave);
        Ok(())
    }
    /// Native picker destination applied through the consumed `preview.snapshot`
    /// command. Failure stays typed and never shows a success toast.
    pub fn snapshot_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let at = self.playback.position()?;
        let protected = self
            .session
            .autosave()
            .map(|autosave| autosave.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        let budget = self.playback.budget().clone();
        match self
            .snapshot
            .save(&mut self.session, budget, at, &path, protected)
        {
            Ok(()) => {
                self.preferences
                    .set("snapshot_directory", serde_json::to_value(path.parent())?)?;
                self.set_toast(
                    self.tr(Text::SnapshotSaved),
                    Some(path.to_string_lossy().into_owned()),
                );
                Ok(())
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Err(error)
            }
        }
    }
    pub fn snapshot_save_cancelled(&mut self) {
        self.snapshot.cancel();
    }
    /// Tests and non-window callers consume the same lifecycle request the
    /// workspace turns into the native picker.
    pub fn take_lifecycle_action(&mut self) -> Option<crate::lifecycle::EditorAction> {
        self.lifecycle_action.take()
    }
    /// Default filename and last directory for the native snapshot dialog.
    pub fn snapshot_save_defaults(&self) -> (String, Option<PathBuf>) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        (
            engine::snapshot::default_name(self.session.project(), at)
                .unwrap_or_else(|_| "snapshot.png".into()),
            self.preferences.snapshot_directory.clone(),
        )
    }
    /// Put the current frame on the OS clipboard through egui's clipboard owner.
    pub fn copy_snapshot(&mut self, ctx: &egui::Context) -> Result<()> {
        let at = self.playback.position()?;
        if let Err(error) = preview::copy(&self.session, at, ctx) {
            self.snapshot.error = Some(match &error {
                Error::Engine(engine::EditError::EmptyProject) => {
                    engine::control::ErrorCode::EmptyProject
                }
                _ => engine::control::ErrorCode::Failed,
            });
            self.error = Some(error.to_string());
            return Err(error);
        }
        self.snapshot.error = None;
        self.set_toast(self.tr(Text::SnapshotCopy), None);
        Ok(())
    }
    pub(in crate::editor) fn seek(&mut self, at: Tick) -> Result<()> {
        self.playback
            .seek(at, self.playback.playing(), self.session.project().clone())?;
        self.remember_view()
    }
    pub(in crate::editor) fn remember_view(&mut self) -> Result<()> {
        if self.session.transaction_active() {
            return Ok(());
        }
        let stored = self.session.metadata().view;
        // Other adapters can issue the engine view command between frontend actions.
        if stored != self.last_view {
            self.pre_anchor_scroll = None;
            self.zoom = (f64::from(DEFAULT_ZOOM) * stored.zoom)
                .clamp(f64::from(MIN_ZOOM_FLOOR), f64::from(MAX_ZOOM))
                as f32;
            self.scroll = (stored.scroll / f64::from(self.zoom)) as f32;
            self.playback.seek(
                stored
                    .playhead
                    .clamp(Tick::ZERO, self.session.project().duration()),
                self.playback.playing(),
                self.session.project().clone(),
            )?;
            self.last_view = stored;
        }
        let view = project::ViewState {
            zoom: f64::from(self.zoom) / f64::from(DEFAULT_ZOOM),
            scroll: (f64::from(self.scroll) * f64::from(self.zoom)).clamp(0.0, 1e9),
            playhead: self.playback.position()?,
        };
        if self.session.metadata().view != view {
            self.session.execute(
                "project.set_view_state",
                serde_json::json!({"zoom":view.zoom,"scroll":view.scroll,"playhead":view.playhead}),
            )?;
        }
        self.last_view = view;
        Ok(())
    }
    pub(in crate::editor) fn seek_to(&mut self, at: Tick) {
        let at = at.clamp(Tick::ZERO, self.session.project().duration());
        if let Err(error) = self.seek(at) {
            self.error = Some(error.to_string());
        }
    }
    pub(in crate::editor) fn step_frames(&mut self, frames: i64) -> Result<()> {
        let frame = self.frame_ticks();
        let at = self.playback.position()?;
        self.seek(
            (at + Tick(frame.0.saturating_mul(frames)))
                .clamp(Tick::ZERO, self.session.project().duration()),
        )
    }
    pub(in crate::editor) fn frame_ticks(&self) -> Tick {
        self.session
            .project()
            .data()
            .canvas
            .fps
            .frame_duration()
            .ok()
            .filter(|frame| *frame > Tick::ZERO)
            .unwrap_or(Tick(1))
    }
    /// Shared transport and preview surface for dedicated review windows.
    pub fn preview_ui(&mut self, ui: &mut egui::Ui) {
        super::show(self, ui);
    }
}

//! Applies player transport, timecode and view requests the frame routes.

use crate::editor::{
    AppCommand, DEFAULT_ZOOM, Editor, Error, MAX_ZOOM, MIN_ZOOM_FLOOR, Result, Text, Tick, Vec2,
};
use crate::services::preview;

impl Editor {
    /// Restart preview/audio work against the current immutable project snapshot.
    /// Old generations are fenced by `Playback::seek`; the position is clamped when
    /// an edit shortens the timeline and the transport state is preserved when able.
    pub(in crate::editor) fn sync_preview(&mut self) {
        let project = self
            .session
            .preview_project()
            .unwrap_or(self.session.project())
            .clone();
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
            self.desk().fail(error.to_string());
        }
    }

    pub(in crate::editor) fn preview_canvas_size(&self) -> [u32; 2] {
        let canvas = &self.session.project().data().canvas;
        [canvas.width, canvas.height]
    }

    pub(in crate::editor) fn format_timecode(&self, at: Tick) -> String {
        let rate = self.session.project().data().canvas.fps;
        crate::editor::panels::player::format_timecode(at, rate)
    }

    pub(in crate::editor) fn current_timecode(&self) -> String {
        self.format_timecode(self.playback.display_position().unwrap_or(Tick::ZERO))
    }

    /// Ask the shared preview to render the timeline panel's hover time, or to
    /// return to the playhead when the pointer left the timeline.
    pub(in crate::editor) fn sync_hover(&mut self) {
        let hover = self.timeline.hover_time();
        if let Err(error) = self.playback.set_hover(hover) {
            self.desk().fail(error.to_string());
        }
    }

    pub(in crate::editor) fn commit_timecode(&mut self) {
        let Some(edit) = self.player.timecode_edit.take() else {
            return;
        };
        let duration = self.session.project().duration();
        let rate = self.session.project().data().canvas.fps;
        match time::parse_timecode(&edit.text, rate) {
            Ok(at) if at >= Tick::ZERO && at <= duration => {
                if let Err(error) = self.dispatch(AppCommand::Seek(at)) {
                    self.desk().fail(error.to_string());
                } else {
                    self.player.timecode_error = None;
                }
            }
            _ => self.player.timecode_error = Some(Text::TimecodeInvalid),
        }
    }

    pub(in crate::editor) fn cancel_timecode(&mut self) {
        self.player.timecode_edit = None;
        self.player.timecode_error = None;
    }

    pub(in crate::editor) fn set_preview_fit(&mut self) -> Result<()> {
        self.player.view.set_fit();
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
            .player
            .view
            .set_zoom(zoom, viewport, canvas)
            .ok_or(Error::Invalid("preview zoom"))?;
        self.preferences
            .set("preview_zoom", serde_json::json!(zoom))?;
        self.preferences
            .set("preview_fit", serde_json::Value::Bool(false))?;
        Ok(())
    }

    pub(in crate::editor) fn preview_zoom_by(&mut self, factor: f32) -> Result<()> {
        let viewport = self.player.viewport;
        let canvas = self.preview_canvas_size();
        let base = if self.player.view.fit {
            self.player.view.scale(viewport, canvas)
        } else {
            self.player.view.zoom
        };
        let target = preview::step_zoom(base, factor);
        if (target - self.player.view.zoom).abs() < f32::EPSILON && !self.player.view.fit {
            return Ok(());
        }
        self.set_preview_zoom(target, viewport, canvas)
    }

    pub(in crate::editor) fn open_snapshot_dialog(&mut self) -> Result<()> {
        if self.session.project().duration() == Tick::ZERO {
            self.player.snapshot.error = Some(engine::control::ErrorCode::EmptyProject);
            return Err(Error::Engine(engine::Error::Artifacts(
                engine::ArtifactError::EmptyProject,
            )));
        }
        self.player.snapshot.open();
        self.lifecycle_action = Some(crate::lifecycle::EditorAction::SnapshotSave);
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
        if stored != self.player.last_view {
            let zoom = (f64::from(DEFAULT_ZOOM) * stored.zoom)
                .clamp(f64::from(MIN_ZOOM_FLOOR), f64::from(MAX_ZOOM))
                as f32;
            self.timeline
                .restore_view(zoom, (stored.scroll / f64::from(zoom)) as f32);
            self.playback.seek(
                stored
                    .playhead
                    .clamp(Tick::ZERO, self.session.project().duration()),
                self.playback.playing(),
                self.session.project().clone(),
            )?;
            self.player.last_view = stored;
        }
        let view = project::ViewState {
            zoom: f64::from(self.timeline.zoom()) / f64::from(DEFAULT_ZOOM),
            scroll: (f64::from(self.timeline.scroll()) * f64::from(self.timeline.zoom()))
                .clamp(0.0, 1e9),
            playhead: self.playback.position()?,
        };
        if self.session.metadata().view != view {
            self.session.apply(engine::Command::SetViewState {
                zoom: view.zoom,
                scroll: view.scroll,
                playhead: view.playhead,
            })?;
        }
        self.player.last_view = view;
        Ok(())
    }

    pub(in crate::editor) fn seek_to(&mut self, at: Tick) {
        let at = at.clamp(Tick::ZERO, self.session.project().duration());
        if let Err(error) = self.seek(at) {
            self.desk().fail(error.to_string());
        }
    }
}

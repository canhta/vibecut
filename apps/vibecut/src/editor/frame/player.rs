//! Applies player transport, timecode and view requests the frame routes.

use crate::editor::{AppCommand, Editor, Error, Result, Text, Tick, Vec2};
use crate::services::preview;

impl Editor {
    /// Restart preview/audio work against the current immutable project snapshot.
    /// Old generations are fenced by `Playback::seek`; the position is clamped when
    /// an edit shortens the timeline and the transport state is preserved when able.
    pub(in crate::editor) fn sync_preview(&mut self) {
        let project = self.display_project();
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
            self.fail(error.to_string());
        }
    }

    /// The project the player renders: the agent's changes while they are previewed, else a
    /// staged preview when one is open, otherwise the committed snapshot.
    pub(in crate::editor) fn display_project(&self) -> std::sync::Arc<project::Project> {
        if let Some(fork) = self.agent.preview_session() {
            return fork.project().clone();
        }
        self.session
            .preview_project()
            .unwrap_or(self.session.project())
            .clone()
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
            self.fail(error.to_string());
        }
    }

    /// The range loop playback holds: the export in/out markers when set and non-empty,
    /// otherwise the whole timeline. `None` while looping is off or the timeline is empty.
    pub(in crate::editor) fn loop_range(&self) -> Option<(Tick, Tick)> {
        let duration = self.display_project().duration();
        let in_out = match self.preferences.export.output.range {
            render::RangeChoice::InOut { start, end } => Some((start, end)),
            _ => None,
        };
        crate::services::playback::loop_window(self.preferences.playback_loop, in_out, duration)
    }

    /// Hold the transport's loop range in step with preferences and the timeline. Runs
    /// every frame; `set_loop` re-seeks only when the range changed.
    pub(in crate::editor) fn sync_loop(&mut self) -> Result<()> {
        let range = self.loop_range();
        let project = self.display_project();
        self.playback.set_loop(range, project)
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
                    self.fail(error.to_string());
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
            return Err(Error::Engine(engine::control::Error::Artifacts(
                engine::ArtifactError::EmptyProject,
            )));
        }
        self.player.snapshot.open();
        self.lifecycle_action = Some(crate::services::lifecycle::EditorAction::SnapshotSave);
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
            self.timeline.adopt_stored_view(stored.zoom, stored.scroll);
            self.playback.seek(
                stored
                    .playhead
                    .clamp(Tick::ZERO, self.session.project().duration()),
                self.playback.playing(),
                self.session.project().clone(),
            )?;
            self.player.last_view = stored;
        }
        let (zoom, scroll) = self.timeline.stored_view();
        let view = project::ViewState {
            zoom,
            scroll,
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
}

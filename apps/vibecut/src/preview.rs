//! Preview-only presentation and the snapshot request adapter.
//!
//! Zoom/pan is app state over retained preview frames; it never enters the
//! project. Snapshot publication is the engine's `preview.snapshot` command,
//! dispatched through control admission so the app adds no render logic.
use crate::{Error, Result, egui};
use engine::{
    ByteBudget, EditError, Session,
    control::{self, Caller, ControlResult, ErrorCode, Policy, Request, ResponseValue},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use time::Tick;

/// Preview scale bounds where 1.0 is 100%.
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 16.0;
/// One explicit zoom step for presets, buttons and the toolbar.
pub const ZOOM_STEP: f32 = 1.25;
pub const MAX_PATH_BYTES: usize = engine::control::MAX_PATH_BYTES;
/// Preset percentages in the toolbar contract.
pub const PRESETS: &[u32] = &[25, 50, 75, 100, 150, 200];

/// A finite zoom inside the admitted range.
pub fn normalize_zoom(zoom: f32) -> Option<f32> {
    (zoom.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(&zoom)).then_some(zoom)
}

/// Zoom factor for one step, clamped to the admitted range.
pub fn step_zoom(zoom: f32, factor: f32) -> f32 {
    let zoom = normalize_zoom(zoom).unwrap_or(1.0);
    if !factor.is_finite() || factor <= 0.0 {
        return zoom;
    }
    (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Scale that fits `canvas` inside `viewport`, preserving aspect ratio.
pub fn fit_scale(viewport: egui::Vec2, canvas: [u32; 2]) -> f32 {
    if canvas[0] == 0 || canvas[1] == 0 || !viewport.is_finite() {
        return 1.0;
    }
    let width = viewport.x.max(0.0) / canvas[0] as f32;
    let height = viewport.y.max(0.0) / canvas[1] as f32;
    width.min(height).max(1e-6)
}

/// Pan bounds keep the scaled canvas covering the viewport; an axis that still
/// fits cannot move.
pub fn pan_limit(viewport: egui::Vec2, canvas: [u32; 2], scale: f32) -> egui::Vec2 {
    if !viewport.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return egui::Vec2::ZERO;
    }
    let size = egui::vec2(canvas[0] as f32 * scale, canvas[1] as f32 * scale);
    egui::vec2(
        ((size.x - viewport.x) * 0.5).max(0.0),
        ((size.y - viewport.y) * 0.5).max(0.0),
    )
}

pub fn clamp_offset(
    offset: egui::Vec2,
    viewport: egui::Vec2,
    canvas: [u32; 2],
    scale: f32,
) -> egui::Vec2 {
    let limit = pan_limit(viewport, canvas, scale);
    egui::vec2(
        offset.x.clamp(-limit.x, limit.x),
        offset.y.clamp(-limit.y, limit.y),
    )
}

/// Preview view state; persisted as the fit flag plus explicit scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub fit: bool,
    pub zoom: f32,
    pub offset: egui::Vec2,
}

impl Default for View {
    fn default() -> Self {
        Self {
            fit: true,
            zoom: 1.0,
            offset: egui::Vec2::ZERO,
        }
    }
}

impl View {
    pub fn new(fit: bool, zoom: f64) -> Self {
        Self {
            fit,
            zoom: normalize_zoom(zoom as f32).unwrap_or(1.0),
            offset: egui::Vec2::ZERO,
        }
    }

    pub fn scale(&self, viewport: egui::Vec2, canvas: [u32; 2]) -> f32 {
        if self.fit {
            fit_scale(viewport, canvas)
        } else {
            normalize_zoom(self.zoom).unwrap_or(1.0)
        }
    }

    /// Panning is an explicit zoom above 100% in this UI; Fit always recentres.
    pub fn can_pan(&self) -> bool {
        !self.fit && self.zoom > 1.0
    }

    pub fn set_fit(&mut self) {
        self.fit = true;
        self.offset = egui::Vec2::ZERO;
    }

    pub fn set_zoom(&mut self, zoom: f32, viewport: egui::Vec2, canvas: [u32; 2]) -> Option<f32> {
        let zoom = normalize_zoom(zoom)?;
        self.fit = false;
        self.zoom = zoom;
        self.offset = clamp_offset(self.offset, viewport, canvas, zoom);
        Some(zoom)
    }

    pub fn zoom_by(&mut self, factor: f32, viewport: egui::Vec2, canvas: [u32; 2]) -> Option<f32> {
        self.set_zoom(step_zoom(self.zoom, factor), viewport, canvas)
    }

    pub fn pan(&mut self, delta: egui::Vec2, viewport: egui::Vec2, canvas: [u32; 2]) -> bool {
        if !delta.is_finite() || !self.can_pan() {
            return false;
        }
        let scale = self.scale(viewport, canvas);
        let before = self.offset;
        self.offset = clamp_offset(self.offset + delta, viewport, canvas, scale);
        self.offset != before
    }

    pub fn recenter(&mut self) {
        self.offset = egui::Vec2::ZERO;
    }
}

/// Trusted composition-root policy: this desktop app owns the destination the
/// native picker returned, exactly like the export dialog.
struct Allow;

impl Policy for Allow {
    fn snapshot_overwrite(&self) -> bool {
        // The native Save dialog already confirmed the destination.
        true
    }

    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}

/// Snapshot save state over the engine command. `saved`/`error` are receipts;
/// the PNG bytes stay in the engine render boundary.
pub struct SnapshotDialog {
    open: bool,
    pub saved: Option<PathBuf>,
    pub error: Option<ErrorCode>,
    runner: Option<JobRunner>,
    requests: AtomicU64,
}

impl Default for SnapshotDialog {
    fn default() -> Self {
        Self {
            open: false,
            saved: None,
            error: None,
            runner: None,
            requests: AtomicU64::new(1),
        }
    }
}

impl SnapshotDialog {
    pub fn open(&mut self) {
        self.open = true;
        self.error = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Cancelling the native picker is a no-op: no command, no toast, no error.
    pub fn cancel(&mut self) {
        self.open = false;
    }

    /// Save through `preview.snapshot`; path and time are the command's typed
    /// parameters. Empty projects and write failures stay typed and never claim
    /// success.
    pub fn save(
        &mut self,
        session: &mut Session,
        budget: ByteBudget,
        at: Tick,
        path: &Path,
        protected: Vec<PathBuf>,
    ) -> Result<()> {
        self.open = false;
        self.saved = None;
        self.error = None;
        if session.project().duration() == Tick::ZERO {
            self.error = Some(ErrorCode::EmptyProject);
            return Err(Error::Engine(EditError::EmptyProject));
        }
        session.protect_paths(protected)?;
        let runner = match &mut self.runner {
            Some(runner) => runner,
            slot @ None => slot.insert(
                JobRunner::new(
                    RunnerConfig {
                        concurrency: [1, 1, 1],
                        queue: 1,
                        retained: 1,
                        network_attempts: 1,
                        backoff: Duration::from_secs(1),
                        progress_interval: Duration::from_millis(20),
                    },
                    budget,
                    Arc::new(SystemClock::default()),
                )
                .map_err(Error::Engine)?,
            ),
        };
        let id = self.requests.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            api_version: control::API_VERSION,
            registry_hash: Some(control::registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: format!("preview-snapshot-{id}"),
            idempotency_key: Some(format!("preview-snapshot-{id}")),
            base_rev: Some(session.revision()),
            command: "preview.snapshot".into(),
            params: serde_json::json!({"time": at.0, "path": path}),
        };
        let response = session.control_request(
            request,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Allow,
            },
            runner,
        )?;
        match response.value {
            ResponseValue::Result(ControlResult::Snapshot { snapshot }) => {
                self.saved = Some(snapshot.path);
                Ok(())
            }
            ResponseValue::Result(_) => {
                self.error = Some(ErrorCode::Failed);
                Err(Error::Invalid("snapshot result"))
            }
            ResponseValue::Error(code) => {
                self.error = Some(code);
                Err(Error::Engine(error_for(code)))
            }
        }
    }
}

fn error_for(code: ErrorCode) -> EditError {
    match code {
        ErrorCode::EmptyProject => EditError::EmptyProject,
        ErrorCode::TransactionActive => EditError::TransactionActive,
        ErrorCode::Permission => EditError::Permission,
        ErrorCode::Limit => EditError::Limit,
        ErrorCode::InvalidParams => EditError::InvalidParams,
        ErrorCode::UnknownCommand => EditError::UnknownCommand,
        ErrorCode::SessionGone => EditError::SessionGone,
        ErrorCode::StaleRevision => EditError::StaleRevision,
        ErrorCode::IdempotencyConflict => EditError::IdempotencyConflict,
        ErrorCode::ReceiptCapacity => EditError::ReceiptCapacity,
        ErrorCode::ResyncRequired => EditError::ResyncRequired,
        ErrorCode::JobNotRetained => EditError::JobNotRetained,
        ErrorCode::Cancelled => EditError::Cancelled,
        ErrorCode::QueueFull => EditError::QueueFull,
        ErrorCode::SchemaMismatch => EditError::SchemaMismatch,
        ErrorCode::Refused | ErrorCode::Failed => EditError::Invalid("snapshot write failed"),
    }
}

/// Decode the same PNG the engine would publish and hand it to egui's OS
/// clipboard boundary. No file is written and the project is untouched.
pub fn copy(session: &Session, at: Tick, ctx: &egui::Context) -> Result<()> {
    let snapshot = engine::snapshot::image(session.project(), at)?;
    let decoded = image::load_from_memory(&snapshot.png)?.into_rgba8();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [decoded.width() as usize, decoded.height() as usize],
        decoded.as_raw(),
    );
    ctx.copy_image(image);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_clamps_at_both_bounds_and_rejects_non_finite_values() {
        let viewport = egui::vec2(640.0, 360.0);
        let canvas = [1920, 1080];
        let mut view = View::default();
        assert_eq!(view.set_zoom(0.01, viewport, canvas), None);
        assert_eq!(view.set_zoom(f32::NAN, viewport, canvas), None);
        assert_eq!(view.set_zoom(f32::INFINITY, viewport, canvas), None);
        assert_eq!(view.set_zoom(0.25, viewport, canvas), Some(0.25));
        assert_eq!(view.set_zoom(16.0, viewport, canvas), Some(16.0));
        assert_eq!(view.zoom_by(2.0, viewport, canvas), Some(16.0));
        assert_eq!(view.zoom_by(0.001, viewport, canvas), Some(0.25));
    }

    #[test]
    fn fit_recentres_and_pan_bounds_keep_the_canvas_covering_the_viewport() {
        let viewport = egui::vec2(640.0, 360.0);
        let canvas = [1920, 1080];
        let mut view = View::default();
        assert!(view.fit);
        assert!((view.scale(viewport, canvas) - (640.0 / 1920.0)).abs() < 1e-6);
        assert!(!view.pan(egui::vec2(10.0, 10.0), viewport, canvas));
        view.set_zoom(2.0, viewport, canvas);
        assert!(view.pan(egui::vec2(10_000.0, 10_000.0), viewport, canvas));
        let limit = pan_limit(viewport, canvas, 2.0);
        assert_eq!(view.offset, limit);
        view.set_fit();
        assert_eq!(view.offset, egui::Vec2::ZERO);
    }

    #[test]
    fn gesture_steps_are_reversible_and_presets_leave_fit() {
        let viewport = egui::vec2(800.0, 450.0);
        let canvas = [800, 450];
        let mut view = View::new(true, 1.0);
        view.set_zoom(1.0, viewport, canvas);
        assert!(view.zoom_by(ZOOM_STEP, viewport, canvas).is_some());
        let up = view.zoom;
        view.zoom_by(1.0 / ZOOM_STEP, viewport, canvas);
        assert!((view.zoom - 1.0).abs() < 1e-6);
        view.set_zoom(PRESETS[5] as f32 / 100.0, viewport, canvas);
        assert!(!view.fit);
        assert_eq!(view.zoom, 2.0);
        assert!(up > 1.0);
    }
}

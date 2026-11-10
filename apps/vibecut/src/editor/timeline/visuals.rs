//! Consumes the engine's cache-only clip display queries.
//!
//! The panel asks for ready tiles, bars or a typed state; this owner opens the
//! process media caches, calls `waveform::display` / `thumbnail_strip::display`
//! and submits the registered generation commands when a display is pending.
//! It never decodes media and never builds a cache key.

use crate::editor::timeline::{AssetId, ItemId, Tick, egui, theme};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
};

/// One ready waveform column, linear amplitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WaveBar {
    pub(in crate::editor) min: f32,
    pub(in crate::editor) max: f32,
    /// Any sample crossed full scale, so the display draws a `> 0 dBFS` tip.
    pub(in crate::editor) over: bool,
}

/// Cache-only waveform state for one clip's visible range.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WaveState {
    Ready(Vec<WaveBar>),
    Pending,
    Failed,
    Unsupported,
}

/// One cached strip tile: the output-time slice it fills and its frame texture.
#[derive(Clone)]
pub(crate) struct StripTile {
    pub(in crate::editor) output: time::TimeRange,
    pub(in crate::editor) texture: egui::TextureHandle,
}

/// Cache-only thumbnail strip state for one clip's visible range.
#[derive(Clone)]
pub(crate) enum StripState {
    Ready(Vec<StripTile>),
    Pending,
    Failed,
    Unsupported,
}

/// A generation command the panel asks the frame to run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum VisualRequest {
    Waveform {
        asset: AssetId,
    },
    Thumbnails {
        item: ItemId,
        visible: time::TimeRange,
        pixels_per_second: f64,
    },
}
impl VisualRequest {
    /// The cache family one in-flight job serves.
    fn key(&self) -> VisualKey {
        match self {
            Self::Waveform { asset } => VisualKey::Waveform(*asset),
            Self::Thumbnails { item, .. } => VisualKey::Thumbnails(*item),
        }
    }
    /// The registered `JobControl` command and its typed parameters.
    pub(in crate::editor) fn command(&self) -> (&'static str, serde_json::Value) {
        match self {
            Self::Waveform { asset } => (
                "media.generate_waveform",
                serde_json::json!({"asset": asset}),
            ),
            Self::Thumbnails {
                item,
                visible,
                pixels_per_second,
            } => (
                "media.generate_clip_thumbnails",
                serde_json::json!({
                    "item": item,
                    "visible": {"start": visible.start().0, "end": visible.end().0},
                    "pixels_per_second": pixels_per_second,
                }),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum VisualKey {
    Waveform(AssetId),
    Thumbnails(ItemId),
}

/// Read-only clip visual queries shared by every timeline area.
pub(crate) trait VisualQuery {
    fn strip(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        pixels_per_second: f64,
    ) -> StripState;
    fn waveform(
        &self,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        columns: usize,
    ) -> WaveState;
    /// Drain the generation commands the last paint asked for.
    fn take_requests(&self) -> Vec<VisualRequest>;
}

/// Reserved texture slots for strip frames; the oldest frame is dropped first.
const MAX_TEXTURES: usize = 512;
/// A cache directory held by another process is retried after this many ticks.
const BUSY_RETRY_TICKS: u32 = 120;
/// Loopback policy for the editor's own registered commands.
struct EditorPolicy;
impl engine::control::Policy for EditorPolicy {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}

enum Access {
    Unopened,
    Ready(Box<RefCell<engine::control::MediaAccess>>),
    Unavailable,
}

/// Texture cache key: asset, cached source position and content identity.
type StripTextures = BTreeMap<(AssetId, Tick, [u8; 32]), egui::TextureHandle>;

/// Production display queries plus the jobs they request.
pub(in crate::editor) struct Media {
    budget: engine::ByteBudget,
    access: RefCell<Access>,
    runner: Option<engine::jobs::JobRunner>,
    /// In-flight generation job -> the display request that submitted it.
    jobs: BTreeMap<engine::jobs::JobId, VisualRequest>,
    /// One request per cache family until its job settles.
    pending: RefCell<BTreeMap<VisualKey, VisualRequest>>,
    /// Terminal generation failures stay typed instead of retrying every frame.
    failed: RefCell<BTreeSet<VisualKey>>,
    queued: RefCell<Vec<VisualRequest>>,
    textures: RefCell<StripTextures>,
    request_id: u64,
    ticks: Cell<u32>,
}

impl Media {
    pub(in crate::editor) fn new(budget: engine::ByteBudget) -> Self {
        Self {
            budget,
            access: RefCell::new(Access::Unopened),
            runner: None,
            jobs: BTreeMap::new(),
            pending: RefCell::new(BTreeMap::new()),
            failed: RefCell::new(BTreeSet::new()),
            queued: RefCell::new(Vec::new()),
            textures: RefCell::new(BTreeMap::new()),
            request_id: 0,
            ticks: Cell::new(0),
        }
    }

    /// Run one display query with the shared caches; `None` while closed.
    fn query<T>(&self, read: impl FnOnce(&mut engine::control::MediaAccess) -> T) -> Option<T> {
        let mut access = self.access.borrow_mut();
        if matches!(*access, Access::Unopened) {
            *access = match engine::control::MediaAccess::open(self.budget.clone()) {
                Ok(opened) => Access::Ready(Box::new(RefCell::new(opened))),
                Err(_) => Access::Unavailable,
            };
        }
        match &*access {
            Access::Ready(opened) => Some(read(&mut opened.borrow_mut())),
            Access::Unopened | Access::Unavailable => None,
        }
    }

    fn texture(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        at: Tick,
        image: &engine::thumbnails::RetainedThumbnail,
    ) -> egui::TextureHandle {
        let key = (asset.id(), at, asset.content_hash().0);
        let mut textures = self.textures.borrow_mut();
        if let Some(texture) = textures.get(&key) {
            return texture.clone();
        }
        if textures.len() >= MAX_TEXTURES
            && let Some(oldest) = textures.keys().next().copied()
        {
            textures.remove(&oldest);
        }
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &image.pixels,
        );
        let texture = ctx.load_texture(
            format!("timeline.strip.{}.{}", asset.id().0, at.0),
            pixels,
            egui::TextureOptions::LINEAR,
        );
        textures.insert(key, texture.clone());
        texture
    }

    /// Queue one generation command per cache family; warm frames ask nothing.
    fn enqueue(&self, request: VisualRequest) {
        let key = request.key();
        let mut pending = self.pending.borrow_mut();
        if pending.contains_key(&key) || self.failed.borrow().contains(&key) {
            return;
        }
        pending.insert(key, request.clone());
        self.queued.borrow_mut().push(request);
    }

    /// Submit one generation command the panel requested.
    pub(in crate::editor) fn generate(
        &mut self,
        session: &mut engine::Session,
        request: VisualRequest,
    ) -> crate::Result<()> {
        let key = request.key();
        if session.transaction_active() {
            // A preview gesture owns the session; the next paint asks again.
            self.pending.borrow_mut().remove(&key);
            return Ok(());
        }
        let (command, params) = request.command();
        let response = match self.control(session, command, params) {
            Ok(response) => response,
            Err(error) => {
                self.pending.borrow_mut().remove(&key);
                return Err(error);
            }
        };
        match response.value {
            engine::control::ResponseValue::Result(
                engine::control::ControlResult::JobAccepted { job, .. },
            ) => {
                self.jobs.insert(job, request);
                Ok(())
            }
            _ => {
                self.pending.borrow_mut().remove(&key);
                self.failed.borrow_mut().insert(key);
                Err(crate::Error::Invalid("clip visual generation refused"))
            }
        }
    }

    fn control(
        &mut self,
        session: &mut engine::Session,
        command: &str,
        params: serde_json::Value,
    ) -> crate::Result<engine::control::Response> {
        let mut access = self.access.borrow_mut();
        if matches!(*access, Access::Unopened) {
            *access = match engine::control::MediaAccess::open(self.budget.clone()) {
                Ok(opened) => Access::Ready(Box::new(RefCell::new(opened))),
                Err(_) => Access::Unavailable,
            };
        }
        let Access::Ready(access) = &*access else {
            return Err(crate::Error::Invalid("media cache unavailable"));
        };
        if self.runner.is_none() {
            let runner = engine::jobs::JobRunner::new(
                engine::jobs::RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 32,
                    retained: 32,
                    network_attempts: 1,
                    backoff: std::time::Duration::from_millis(100),
                    progress_interval: std::time::Duration::from_millis(50),
                },
                self.budget.clone(),
                std::sync::Arc::new(engine::jobs::SystemClock::default()),
            )?;
            self.runner = Some(runner);
        }
        let runner = self
            .runner
            .as_mut()
            .ok_or(crate::Error::Invalid("visual job runner"))?;
        self.request_id = self.request_id.saturating_add(1);
        let id = self.request_id;
        let request = engine::control::Request {
            api_version: engine::control::API_VERSION,
            registry_hash: Some(engine::control::registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: format!("timeline-visual-{id}"),
            idempotency_key: Some(format!("timeline-visual-{id}")),
            base_rev: Some(session.revision()),
            command: command.to_owned(),
            params,
        };
        let response = session.control_request(
            request,
            engine::control::Caller {
                principal: "vibecut-timeline",
                external: false,
                policy: &EditorPolicy,
                fonts: None,
                media: Some(access.as_ref()),
            },
            runner,
        )?;
        Ok(response)
    }

    /// Advance generation jobs and publish finished outputs through `job_status`.
    pub(in crate::editor) fn tick(&mut self, session: &mut engine::Session) -> crate::Result<()> {
        self.ticks.set(self.ticks.get().wrapping_add(1));
        let retry = self.ticks.get().is_multiple_of(BUSY_RETRY_TICKS)
            && matches!(*self.access.borrow(), Access::Unavailable);
        if retry {
            *self.access.borrow_mut() = Access::Unopened;
        }
        if let Some(runner) = &mut self.runner {
            runner.tick()?;
        }
        let ids: Vec<engine::jobs::JobId> = self.jobs.keys().copied().collect();
        for id in ids {
            let state = self
                .runner
                .as_ref()
                .and_then(|runner| runner.snapshot(id))
                .map(|snapshot| snapshot.state);
            let Some(state) = state else {
                self.settle(id, None);
                continue;
            };
            if !matches!(
                state,
                engine::jobs::JobState::Completed
                    | engine::jobs::JobState::Cancelled
                    | engine::jobs::JobState::Failed
            ) {
                continue;
            }
            let result = self.status(session, id);
            self.settle(id, Some((state, result)));
        }
        Ok(())
    }

    fn status(
        &mut self,
        session: &mut engine::Session,
        job: engine::jobs::JobId,
    ) -> crate::Result<()> {
        let response = self.control(session, "job_status", serde_json::json!({"job": job}))?;
        match response.value {
            engine::control::ResponseValue::Result(engine::control::ControlResult::Job {
                ..
            }) => Ok(()),
            _ => Err(crate::Error::Invalid("clip visual job status")),
        }
    }

    fn settle(
        &mut self,
        job: engine::jobs::JobId,
        outcome: Option<(engine::jobs::JobState, crate::Result<()>)>,
    ) {
        let Some(request) = self.jobs.remove(&job) else {
            return;
        };
        self.pending.borrow_mut().remove(&request.key());
        if let Some((state, result)) = outcome
            && (state == engine::jobs::JobState::Failed || result.is_err())
        {
            self.failed.borrow_mut().insert(request.key());
        }
    }
}

impl VisualQuery for Media {
    fn strip(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        pixels_per_second: f64,
    ) -> StripState {
        let key = VisualKey::Thumbnails(item.id);
        if self.failed.borrow().contains(&key) {
            return StripState::Failed;
        }
        let Some(state) = self.query(|access| {
            access.thumbnail_strip_display(asset, item, visible, pixels_per_second)
        }) else {
            return StripState::Pending;
        };
        match state {
            Ok(engine::thumbnail_strip::ThumbnailStripState::Ready(display)) => StripState::Ready(
                display
                    .tiles()
                    .iter()
                    .map(|tile| StripTile {
                        output: tile.output,
                        texture: self.texture(ctx, asset, tile.at, &tile.image),
                    })
                    .collect(),
            ),
            Ok(engine::thumbnail_strip::ThumbnailStripState::Pending) => {
                self.enqueue(VisualRequest::Thumbnails {
                    item: item.id,
                    visible,
                    pixels_per_second,
                });
                StripState::Pending
            }
            Ok(engine::thumbnail_strip::ThumbnailStripState::Failed(_)) => {
                self.failed.borrow_mut().insert(key);
                StripState::Failed
            }
            Ok(engine::thumbnail_strip::ThumbnailStripState::Unsupported) => {
                StripState::Unsupported
            }
            Err(_) => {
                self.failed.borrow_mut().insert(key);
                StripState::Failed
            }
        }
    }

    fn waveform(
        &self,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        columns: usize,
    ) -> WaveState {
        let key = VisualKey::Waveform(asset.id());
        if self.failed.borrow().contains(&key) {
            return WaveState::Failed;
        }
        let Some(state) =
            self.query(|access| access.waveform_display(asset, item, visible, columns))
        else {
            return WaveState::Pending;
        };
        match state {
            Ok(engine::waveform::WaveformState::Ready(display)) => WaveState::Ready(
                display
                    .bars()
                    .iter()
                    .map(|bar| WaveBar {
                        min: bar.min,
                        max: bar.max,
                        over: bar.over,
                    })
                    .collect(),
            ),
            Ok(engine::waveform::WaveformState::Pending) => {
                self.enqueue(VisualRequest::Waveform { asset: asset.id() });
                WaveState::Pending
            }
            Ok(engine::waveform::WaveformState::Failed(_)) => {
                self.failed.borrow_mut().insert(key);
                WaveState::Failed
            }
            Ok(engine::waveform::WaveformState::Unsupported) => WaveState::Unsupported,
            Err(_) => {
                self.failed.borrow_mut().insert(key);
                WaveState::Failed
            }
        }
    }

    fn take_requests(&self) -> Vec<VisualRequest> {
        std::mem::take(&mut *self.queued.borrow_mut())
    }
}

/// The warning ink for an over-full-scale waveform tip.
pub(in crate::editor) fn clip_tip(dark: bool) -> egui::Color32 {
    theme::warning(dark)
}

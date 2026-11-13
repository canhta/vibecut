#![allow(dead_code, clippy::unwrap_used, clippy::panic)]
use render::{Error, ExecutionHooks, MemoryKind, Reservation, Result, Stage};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};
#[derive(Clone)]
pub struct Hooks {
    pub state: Arc<AtomicU8>,
    pub used: Arc<AtomicU64>,
    pub peak: Arc<AtomicU64>,
    pub requested: Arc<AtomicU64>,
    pub limit: u64,
    pub event: Option<Arc<dyn Fn(Stage) -> Result<()> + Send + Sync>>,
}
impl Default for Hooks {
    fn default() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(0)),
            used: Arc::new(AtomicU64::new(0)),
            peak: Arc::new(AtomicU64::new(0)),
            requested: Arc::new(AtomicU64::new(0)),
            limit: 2 << 30,
            event: None,
        }
    }
}
struct Lease {
    used: Arc<AtomicU64>,
    bytes: u64,
}
impl Reservation for Lease {}
impl Drop for Lease {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.state.load(Ordering::SeqCst) == 1
    }
    fn begin_publish(&self) -> Result<()> {
        match self
            .state
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) | Err(2) => Ok(()),
            _ => Err(Error::Cancelled),
        }
    }
    fn reserve(&self, _: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
        self.requested.fetch_max(bytes, Ordering::SeqCst);
        let mut used = self.used.load(Ordering::SeqCst);
        loop {
            let next = used.checked_add(bytes).ok_or(Error::Budget {
                requested: bytes,
                limit: self.limit,
            })?;
            if next > self.limit {
                return Err(Error::Budget {
                    requested: bytes,
                    limit: self.limit,
                });
            }
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    self.peak.fetch_max(next, Ordering::SeqCst);
                    return Ok(Box::new(Lease {
                        used: self.used.clone(),
                        bytes,
                    }));
                }
                Err(current) => used = current,
            }
        }
    }
    fn progress(&self, s: Stage, _: f32) -> Result<()> {
        if let Some(event) = &self.event {
            event(s)?;
        }
        Ok(())
    }
}
pub use testkit::oracle::command as tool_command;
pub use testkit::oracle::{ffmpeg, probe, rgba};

pub fn tools() -> bool {
    testkit::oracle::require("render oracle")
}

pub fn root() -> PathBuf {
    testkit::fixture::dir("render")
}

pub fn tick(samples: i64, rate: i64) -> time::Tick {
    time::Tick::from_units(samples, rate, time::Rounding::Nearest).unwrap()
}
pub fn solid_project(seconds: i64, width: u32, height: u32, color: [u8; 4]) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::item(
                project::ItemId(0),
                project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color(color),
                }),
            )
            .source(time::SourceRange::new(time::Tick::ZERO, tick(seconds, 1)).unwrap())
            .build(),
        ],
    });
    project::Project::new(data).unwrap()
}
/// Fetched-sticker fixture: three downloaded files (a red, a green and a blue square) and one id
/// whose file is not downloaded. Every test lends the same source, so tests share it safely.
pub struct FetchedStickers;
impl FetchedStickers {
    pub const RED: &'static str = "noto:1f600";
    pub const GREEN: &'static str = "lucide:heart";
    pub const BLUE: &'static str = "phosphor:star";
    pub const NOT_DOWNLOADED: &'static str = "noto:1f601";

    pub fn lend() {
        render::lend_sticker_source(Arc::new(Self));
    }
}
impl render::StickerSource for FetchedStickers {
    fn svg(&self, sticker: &str) -> std::result::Result<Vec<u8>, render::StickerFault> {
        let color = match sticker {
            Self::RED => "#ff0000",
            Self::GREEN => "#00ff00",
            Self::BLUE => "#0000ff",
            Self::NOT_DOWNLOADED => return Err(render::StickerFault::NotDownloaded),
            _ => return Err(render::StickerFault::Unknown),
        };
        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"{color}\"/></svg>"
        )
        .into_bytes())
    }
}
pub struct Empty;
impl render::SourceProvider for Empty {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<render::UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: time::Tick,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<render::Image>> {
        panic!("unexpected video read")
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        _: usize,
        _: u32,
        _: &dyn ExecutionHooks,
    ) -> Result<render::AudioBlock> {
        panic!("unexpected audio read")
    }
}

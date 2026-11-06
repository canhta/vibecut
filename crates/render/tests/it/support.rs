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
        items: vec![project::Item {
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: None,
            source_range: time::SourceRange::new(time::Tick::ZERO, tick(seconds, 1)).unwrap(),
            start: time::Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color(color),
            }),
        }],
    });
    project::Project::new(data).unwrap()
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

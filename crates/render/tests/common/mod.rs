#![allow(dead_code, clippy::unwrap_used, clippy::panic)]
use render::{Error, ExecutionHooks, MemoryKind, Reservation, Result, Stage};
use std::{
    path::{Path, PathBuf},
    process::Command,
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
pub fn tools() -> bool {
    let available = ["ffmpeg", "ffprobe"].iter().all(|tool| {
        match tool_command(tool).arg("-version").output() {
            Ok(output) => {
                assert!(output.status.success(), "found {tool} but it failed");
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("cannot run {tool}: {e}"),
        }
    });
    if !available {
        assert!(
            std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
            "required ffmpeg/ffprobe missing"
        );
        eprintln!("SKIP render oracle: ffmpeg/ffprobe missing");
    }
    available
}
pub fn root() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/render");
    std::fs::create_dir_all(&path).unwrap();
    path
}
pub fn ffmpeg(args: &[&std::ffi::OsStr]) {
    let output = tool_command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
pub fn rgba(path: &Path) -> Vec<u8> {
    let output = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    output.stdout
}
pub fn probe(path: &Path) -> serde_json::Value {
    let output = tool_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
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
    data.timeline.tracks.push(project::Track {
        id: project::TrackId(0),
        name: "".into(),
        kind: project::TrackKind::Overlay,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
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

pub fn tool_command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    c
}

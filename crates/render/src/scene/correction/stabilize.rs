//! Turns per-source-frame camera poses into the warp that steadies one clip.
use crate::{ExecutionHooks, Image, Result, check};
use project::StabilizeLevel;
use std::sync::Arc;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// Frames one track may describe (2.4 hours at 30 fps); bounds the encoded track to about 6 MiB.
pub const MAX_FRAMES: usize = 1 << 18;
/// Largest zoom a level may ask for; beyond it the border shows rather than the picture vanishing.
pub const MAX_ZOOM: f64 = 2.0;
const MAGIC: &[u8; 4] = b"VCST";
const VERSION: u32 = 1;
const HEADER: usize = 20;
/// Bytes per frame in the encoding: an i64 time and four f32 pose terms.
const FRAME_BYTES: usize = 24;
/// The smoothing window reaches this many sigmas either side of a frame.
const REACH: f64 = 3.0;
/// Analysis-grid pixels kept between the steadied picture and the frame edge, so bilinear taps at
/// the border never reach outside the decoded frame.
const MARGIN: f64 = 1.0;

/// The tracks of the stabilized items of a project, by item.
pub type Tracks = std::collections::BTreeMap<project::ItemId, Arc<Track>>;

/// A similarity of the plane about the frame centre in centred coordinates:
/// `p' = [[a, -b], [b, a]] p + t`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sim {
    a: f64,
    b: f64,
    tx: f64,
    ty: f64,
}
impl Sim {
    /// `self` applied after `first`.
    fn after(&self, first: &Self) -> Self {
        Self {
            a: self.a * first.a - self.b * first.b,
            b: self.b * first.a + self.a * first.b,
            tx: self.a * first.tx - self.b * first.ty + self.tx,
            ty: self.b * first.tx + self.a * first.ty + self.ty,
        }
    }
    fn inverse(&self) -> Self {
        let d = self.a * self.a + self.b * self.b;
        let (ia, ib) = (self.a / d, -self.b / d);
        Self {
            a: ia,
            b: ib,
            tx: -(ia * self.tx - ib * self.ty),
            ty: -(ib * self.tx + ia * self.ty),
        }
    }
}

/// The camera pose of one source frame relative to the first, in analysis-grid pixels about the
/// frame centre: where the first frame's content sits in this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub tx: f64,
    pub ty: f64,
    /// Radians, accumulated so it never wraps.
    pub angle: f64,
    /// Natural log of the scale.
    pub ln_scale: f64,
}
impl Pose {
    pub const IDENTITY: Self = Self {
        tx: 0.0,
        ty: 0.0,
        angle: 0.0,
        ln_scale: 0.0,
    };
    fn sim(self) -> Sim {
        let scale = self.ln_scale.exp();
        Sim {
            a: scale * self.angle.cos(),
            b: scale * self.angle.sin(),
            tx: self.tx,
            ty: self.ty,
        }
    }
    /// The pose after the next frame moved the content by the similarity
    /// `p' = [[a, -b], [b, a]] p + t` (centred grid pixels).
    pub fn then(self, a: f64, b: f64, tx: f64, ty: f64) -> Self {
        let step = Sim { a, b, tx, ty };
        let composed = step.after(&self.sim());
        Self {
            tx: composed.tx,
            ty: composed.ty,
            angle: self.angle + b.atan2(a),
            ln_scale: self.ln_scale + 0.5 * (a * a + b * b).ln(),
        }
    }
    fn finite(self) -> bool {
        [self.tx, self.ty, self.angle, self.ln_scale]
            .iter()
            .all(|v| v.is_finite())
    }
}

/// Why a track or its encoding was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid stabilization track: {0}")]
pub struct TrackError(pub &'static str);

/// The measured camera poses of every frame of one source, in source time. It is derived data
/// that the engine computes once per source and caches; levels, trims and splits only read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    grid: [u32; 2],
    times: Vec<Tick>,
    poses: Vec<[f32; 4]>,
}
impl Track {
    /// A track over `frames` at strictly increasing source times, with finite poses measured on an
    /// analysis grid of `grid` pixels.
    pub fn new(
        grid: [u32; 2],
        times: Vec<Tick>,
        poses: &[Pose],
    ) -> std::result::Result<Self, TrackError> {
        if grid[0] == 0 || grid[1] == 0 || grid[0] > 1 << 14 || grid[1] > 1 << 14 {
            return Err(TrackError("grid"));
        }
        if times.is_empty() || times.len() > MAX_FRAMES || times.len() != poses.len() {
            return Err(TrackError("frame count"));
        }
        if times
            .windows(2)
            .any(|pair| matches!(pair, [before, after] if before >= after))
        {
            return Err(TrackError("frame order"));
        }
        if poses.iter().any(|pose| !pose.finite()) {
            return Err(TrackError("pose"));
        }
        Ok(Self {
            grid,
            times,
            poses: poses
                .iter()
                .map(|p| [p.tx as f32, p.ty as f32, p.angle as f32, p.ln_scale as f32])
                .collect(),
        })
    }
    /// Analysis grid the translations are measured in.
    pub fn grid(&self) -> [u32; 2] {
        self.grid
    }
    /// Frame times in ascending source order.
    pub fn times(&self) -> &[Tick] {
        &self.times
    }
    pub fn len(&self) -> usize {
        self.times.len()
    }
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
    /// The raw pose of frame `index`.
    pub fn pose(&self, index: usize) -> Option<Pose> {
        self.poses.get(index).map(|p| Pose {
            tx: f64::from(p[0]),
            ty: f64::from(p[1]),
            angle: f64::from(p[2]),
            ln_scale: f64::from(p[3]),
        })
    }
    /// Deterministic little-endian bytes: the same track always encodes to the same bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + FRAME_BYTES * self.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.grid[0].to_le_bytes());
        out.extend_from_slice(&self.grid[1].to_le_bytes());
        out.extend_from_slice(&(self.len() as u32).to_le_bytes());
        for time in &self.times {
            out.extend_from_slice(&time.0.to_le_bytes());
        }
        for pose in &self.poses {
            for term in pose {
                out.extend_from_slice(&term.to_le_bytes());
            }
        }
        out
    }
    /// Read a track back, refusing any length, order or value `new` would refuse.
    pub fn decode(bytes: &[u8]) -> std::result::Result<Self, TrackError> {
        let length = TrackError("length");
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        if bytes.get(..4) != Some(MAGIC.as_slice()) || word(4) != Some(VERSION) {
            return Err(TrackError("format"));
        }
        let grid = [word(8).ok_or(length)?, word(12).ok_or(length)?];
        let count = word(16).ok_or(length)? as usize;
        if count == 0 || count > MAX_FRAMES || bytes.len() != HEADER + FRAME_BYTES * count {
            return Err(length);
        }
        let mut times = Vec::with_capacity(count);
        let mut poses = Vec::with_capacity(count);
        let poses_at = HEADER + 8 * count;
        for index in 0..count {
            let at = HEADER + 8 * index;
            let raw = bytes.get(at..at + 8).ok_or(length)?;
            times.push(Tick(i64::from_le_bytes(
                raw.try_into().map_err(|_| length)?,
            )));
            let term = |n: usize| -> std::result::Result<f64, TrackError> {
                let from = poses_at + 16 * index + 4 * n;
                let raw = bytes.get(from..from + 4).ok_or(length)?;
                Ok(f64::from(f32::from_le_bytes(
                    raw.try_into().map_err(|_| length)?,
                )))
            };
            poses.push(Pose {
                tx: term(0)?,
                ty: term(1)?,
                angle: term(2)?,
                ln_scale: term(3)?,
            });
        }
        Self::new(grid, times, &poses)
    }
    /// Time of frame `index` in seconds; a missing frame is infinitely late.
    fn seconds(&self, index: usize) -> f64 {
        self.times.get(index).map_or(f64::INFINITY, |tick| {
            tick.0 as f64 / TICKS_PER_SECOND as f64
        })
    }
    /// Index of the frame covering `at`: the last frame at or before it, the first when none is.
    fn covering(&self, at: Tick) -> usize {
        self.times
            .partition_point(|time| *time <= at)
            .saturating_sub(1)
    }
    /// The steadied warp of every frame an item with this source range shows at `level`, and the
    /// one fixed zoom that keeps the picture inside the frame for all of them. The zoom is a
    /// property of the range, so a trim changes it and a switch of level needs no new analysis.
    pub fn stabilizer(&self, level: StabilizeLevel, range: SourceRange) -> Stabilizer {
        let first = self.covering(range.start());
        let end = self.times.partition_point(|time| *time < range.end());
        let last = end.saturating_sub(1).max(first);
        let sigma = level.sigma_seconds();
        let centre = [f64::from(self.grid[0]) / 2.0, f64::from(self.grid[1]) / 2.0];
        let mut maps = Vec::with_capacity(last - first + 1);
        let mut window_start = 0;
        for index in first..=last {
            let now = self.seconds(index);
            while self.seconds(window_start) < now - REACH * sigma {
                window_start += 1;
            }
            let mut sum = [0.0f64; 4];
            let mut weight = 0.0f64;
            for other in window_start..self.times.len() {
                let distance = self.seconds(other) - now;
                if distance > REACH * sigma {
                    break;
                }
                let w = (-(distance * distance) / (2.0 * sigma * sigma)).exp();
                let pose = self.pose(other).unwrap_or(Pose::IDENTITY);
                sum[0] += w * pose.tx;
                sum[1] += w * pose.ty;
                sum[2] += w * pose.angle;
                sum[3] += w * pose.ln_scale;
                weight += w;
            }
            let smooth = Pose {
                tx: sum[0] / weight,
                ty: sum[1] / weight,
                angle: sum[2] / weight,
                ln_scale: sum[3] / weight,
            };
            let measured = self.pose(index).unwrap_or(Pose::IDENTITY);
            // Content measured at `measured` is shown at `smooth`: output pixel q samples the
            // frame at `measured after smooth^-1`.
            maps.push(measured.sim().after(&smooth.sim().inverse()));
        }
        let zoom = maps
            .iter()
            .fold(1.0f64, |zoom, map| zoom.max(needed_zoom(map, centre)));
        Stabilizer {
            grid: self.grid,
            times: self.times.get(first..=last).unwrap_or_default().to_vec(),
            maps,
            zoom,
        }
    }
}

/// The smallest zoom about the frame centre at which the sampling map `map` keeps all four output
/// corners inside the frame; `MAX_ZOOM` when no zoom does.
fn needed_zoom(map: &Sim, half: [f64; 2]) -> f64 {
    // With zoom z an output offset q reads the frame at `A q / z + t`, so each axis needs
    // `reach / z + |t| <= half`, where `reach` is the farthest corner of the rotated rectangle.
    let reach = [
        half[0] * map.a.abs() + half[1] * map.b.abs(),
        half[0] * map.b.abs() + half[1] * map.a.abs(),
    ];
    let offset = [map.tx.abs(), map.ty.abs()];
    let mut zoom = 1.0f64;
    for ((half, reach), offset) in half.iter().zip(reach).zip(offset) {
        let room = (half - MARGIN).max(half * 0.5) - offset;
        if room <= 0.0 {
            return MAX_ZOOM;
        }
        zoom = zoom.max(reach / room);
    }
    zoom.min(MAX_ZOOM)
}

/// The warp of every frame one clip shows plus its fixed zoom.
#[derive(Clone, Debug)]
pub struct Stabilizer {
    grid: [u32; 2],
    times: Vec<Tick>,
    maps: Vec<Sim>,
    zoom: f64,
}
impl Stabilizer {
    /// The one zoom every frame of the clip is scaled by.
    pub fn zoom(&self) -> f64 {
        self.zoom
    }
    /// Frames the stabilizer covers.
    pub fn len(&self) -> usize {
        self.maps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
    /// The warp of the frame covering source time `at`; times outside the clip use its nearest
    /// frame.
    pub fn warp(&self, at: Tick) -> Option<Warp> {
        let index = self
            .times
            .partition_point(|time| *time <= at)
            .saturating_sub(1);
        let map = self.maps.get(index)?;
        Some(Warp {
            grid: self.grid,
            map: *map,
            zoom: self.zoom,
        })
    }
    /// Corner positions, in the source frame's grid pixels about its centre, that the four output
    /// corners read at source time `at`. Used by oracles that bound the picture inside the frame.
    pub fn corner_sources(&self, at: Tick) -> Option<[[f64; 2]; 4]> {
        let warp = self.warp(at)?;
        let half = [f64::from(self.grid[0]) / 2.0, f64::from(self.grid[1]) / 2.0];
        Some(
            [
                [-half[0], -half[1]],
                [half[0], -half[1]],
                [half[0], half[1]],
                [-half[0], half[1]],
            ]
            .map(|corner| warp.source_of(corner, 1.0)),
        )
    }
}

/// One frame's sampling map with the clip's zoom.
#[derive(Clone, Copy, Debug)]
pub struct Warp {
    grid: [u32; 2],
    map: Sim,
    zoom: f64,
}
impl Warp {
    /// Where output offset `q` (about the centre, in image pixels) reads the source frame, in the
    /// same units. `scale` is image pixels per analysis-grid pixel.
    pub fn source_of(&self, q: [f64; 2], scale: f64) -> [f64; 2] {
        [
            (self.map.a * q[0] - self.map.b * q[1]) / self.zoom + scale * self.map.tx,
            (self.map.b * q[0] + self.map.a * q[1]) / self.zoom + scale * self.map.ty,
        ]
    }
    /// Where a point of the steadied picture, in pixels of a `width` x `height` frame, reads the
    /// decoded source frame, in the same units. A person matte measured on the source frame is
    /// mapped through this so it stays on the subject of a steadied clip.
    pub fn frame_point(&self, p: [f64; 2], width: u32, height: u32) -> [f64; 2] {
        let scale = f64::from(width) / f64::from(self.grid[0]);
        let half = [f64::from(width) / 2.0, f64::from(height) / 2.0];
        let source = self.source_of([p[0] - half[0], p[1] - half[1]], scale);
        [source[0] + half[0], source[1] + half[1]]
    }
    /// The frame steadied: every output pixel sampled where the warp says. `None` when the warp is
    /// the identity, so an unshaken, unzoomed frame costs nothing.
    pub fn apply(&self, image: &Image, hooks: &dyn ExecutionHooks) -> Result<Option<Image>> {
        let identity = (self.map.a - 1.0).abs() < 1e-9
            && self.map.b.abs() < 1e-9
            && self.map.tx.abs() < 1e-9
            && self.map.ty.abs() < 1e-9
            && (self.zoom - 1.0).abs() < 1e-9;
        if identity {
            return Ok(None);
        }
        let scale = f64::from(image.width) / f64::from(self.grid[0]);
        let mut out = Image::new(image.width, image.height, [0.0; 4], hooks)?;
        let half = [f64::from(image.width) / 2.0, f64::from(image.height) / 2.0];
        for (y, row) in out.pixels.chunks_mut(image.width as usize).enumerate() {
            check(hooks)?;
            for (x, pixel) in row.iter_mut().enumerate() {
                let q = [x as f64 + 0.5 - half[0], y as f64 + 0.5 - half[1]];
                let source = self.source_of(q, scale);
                *pixel = crate::image::sample(
                    image,
                    (source[0] + half[0] - 0.5) as f32,
                    (source[1] + half[1] - 0.5) as f32,
                );
            }
        }
        Ok(Some(out))
    }
}

/// One item's stabilizer, kept while the item's track, level and range stay the same.
pub(crate) struct Cached {
    pub(crate) track: Arc<Track>,
    pub(crate) level: StabilizeLevel,
    pub(crate) range: SourceRange,
    pub(crate) stabilizer: Arc<Stabilizer>,
}

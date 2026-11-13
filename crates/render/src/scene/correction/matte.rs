//! Turns a source's stored person matte into the coverage that removes a clip's background.
use crate::effects::{
    CpuEffects, EffectCompositor, EffectInputs, EffectPass, SampleInputs, Uniform,
};
use crate::{Error, ExecutionHooks, Image, Result, check};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Tick;

/// Side of the square matte plane: the segmentation model's own resolution.
pub const SIDE: usize = 192;
/// Cells of one plane.
pub const CELLS: usize = SIDE * SIDE;
/// Frames one matte may describe (36 minutes at 30 fps).
pub const MAX_FRAMES: usize = 1 << 16;
/// Largest encoded matte: frame records plus compressed planes.
pub const MAX_BYTES: usize = 128 << 20;
/// Versioned storage and application recipe; the engine puts it in every cache key.
pub const RECIPE: &str = "person-matte-v1:u8-plane192:packbits:feather-box2x-r8:bilinear-stretch";
/// Shader entry of the application pass.
pub const ENTRY: &str = "person_matte";
/// Box radius, in plane cells, at the largest feather. A decision, not measured parity.
const BLUR_RADIUS_AT_MAX: f32 = 8.0;
const BLUR_PASSES: usize = 2;
const MAGIC: &[u8; 4] = b"VCPM";
const VERSION: u32 = 1;
/// Magic, version and frame count.
const HEADER: usize = 12;
/// Per frame: an i64 time and the u32 length of its compressed plane.
const RECORD: usize = 12;

/// The stored mattes of the matted items of a project, by item.
pub type Mattes = BTreeMap<project::ItemId, Arc<MatteTrack>>;

/// Why a matte or its encoding was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid person matte: {0}")]
pub struct MatteError(pub &'static str);

/// The person probability of every frame of one source, quantized to one byte per plane cell and
/// kept compressed. It is derived data that the engine computes once per source and caches;
/// trims, splits and the feather only read it.
#[derive(Clone, Debug, PartialEq)]
pub struct MatteTrack {
    times: Vec<Tick>,
    /// End offset of each frame's compressed plane in `data`.
    ends: Vec<usize>,
    data: Vec<u8>,
}

/// Collects one source's planes in source order within the frame and byte bounds.
#[derive(Debug, Default)]
pub struct MatteBuilder {
    times: Vec<Tick>,
    ends: Vec<usize>,
    data: Vec<u8>,
}
impl MatteBuilder {
    /// Append the plane of the frame at `at`: `CELLS` bytes, row-major, 255 for certainly a person.
    pub fn push(&mut self, at: Tick, plane: &[u8]) -> std::result::Result<(), MatteError> {
        if plane.len() != CELLS {
            return Err(MatteError("plane size"));
        }
        if self.times.last().is_some_and(|last| *last >= at) {
            return Err(MatteError("frame order"));
        }
        if self.times.len() >= MAX_FRAMES {
            return Err(MatteError("frame count"));
        }
        pack(plane, &mut self.data);
        if HEADER + RECORD * (self.times.len() + 1) + self.data.len() > MAX_BYTES {
            return Err(MatteError("encoded size"));
        }
        self.times.push(at);
        self.ends.push(self.data.len());
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.times.len()
    }
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
    pub fn finish(self) -> std::result::Result<MatteTrack, MatteError> {
        if self.times.is_empty() {
            return Err(MatteError("frame count"));
        }
        Ok(MatteTrack {
            times: self.times,
            ends: self.ends,
            data: self.data,
        })
    }
}

impl MatteTrack {
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
    /// Index of the frame covering `at`: the last frame at or before it, the first when none is.
    pub fn covering(&self, at: Tick) -> usize {
        self.times
            .partition_point(|time| *time <= at)
            .saturating_sub(1)
    }
    /// The plane of frame `index`, `CELLS` bytes.
    pub fn plane(&self, index: usize) -> Result<Vec<u8>> {
        let end = *self
            .ends
            .get(index)
            .ok_or(Error::Invalid("person matte frame"))?;
        let start = index
            .checked_sub(1)
            .and_then(|before| self.ends.get(before).copied())
            .unwrap_or(0);
        let packed = self
            .data
            .get(start..end)
            .ok_or(Error::Invalid("person matte frame"))?;
        unpack(packed).ok_or(Error::Invalid("person matte frame"))
    }
    /// Deterministic little-endian bytes: the same track always encodes to the same bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + RECORD * self.len() + self.data.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.len() as u32).to_le_bytes());
        let mut start = 0;
        for (time, end) in self.times.iter().zip(&self.ends) {
            out.extend_from_slice(&time.0.to_le_bytes());
            out.extend_from_slice(&((end - start) as u32).to_le_bytes());
            start = *end;
        }
        out.extend_from_slice(&self.data);
        out
    }
    /// Read a matte back, refusing any length, order or plane `MatteBuilder` would refuse. Every
    /// plane's compressed stream is checked to expand to exactly one plane without expanding it.
    pub fn decode(bytes: &[u8]) -> std::result::Result<Self, MatteError> {
        let length = MatteError("length");
        if bytes.len() > MAX_BYTES {
            return Err(length);
        }
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        if bytes.get(..4) != Some(MAGIC.as_slice()) || word(4) != Some(VERSION) {
            return Err(MatteError("format"));
        }
        let count = word(8).ok_or(length)? as usize;
        if count == 0 || count > MAX_FRAMES {
            return Err(MatteError("frame count"));
        }
        let records = HEADER + RECORD * count;
        let data = bytes.get(records..).ok_or(length)?;
        let mut times = Vec::with_capacity(count);
        let mut ends = Vec::with_capacity(count);
        let mut end = 0usize;
        for index in 0..count {
            let at = HEADER + RECORD * index;
            let time = Tick(i64::from_le_bytes(
                bytes
                    .get(at..at + 8)
                    .ok_or(length)?
                    .try_into()
                    .map_err(|_| length)?,
            ));
            if times.last().is_some_and(|last| *last >= time) {
                return Err(MatteError("frame order"));
            }
            let size = word(at + 8).ok_or(length)? as usize;
            let start = end;
            end = start.checked_add(size).ok_or(length)?;
            if !packed_plane(data.get(start..end).ok_or(length)?) {
                return Err(MatteError("plane"));
            }
            times.push(time);
            ends.push(end);
        }
        if end != data.len() {
            return Err(length);
        }
        Ok(Self {
            times,
            ends,
            data: data.to_vec(),
        })
    }
}

/// PackBits: a header `n` below 128 copies the next `n + 1` bytes; `n` from 128 repeats the next
/// byte `n - 126` times (2..=129). Runs of two or more equal bytes always become runs, so one
/// plane always packs to the same bytes.
fn pack(plane: &[u8], out: &mut Vec<u8>) {
    let mut index = 0;
    let mut literal: Vec<u8> = Vec::with_capacity(128);
    let flush = |literal: &mut Vec<u8>, out: &mut Vec<u8>| {
        if !literal.is_empty() {
            out.push((literal.len() - 1) as u8);
            out.append(literal);
        }
    };
    while let Some(&value) = plane.get(index) {
        let run = plane
            .get(index..)
            .unwrap_or_default()
            .iter()
            .take(129)
            .take_while(|other| **other == value)
            .count();
        if run >= 2 {
            flush(&mut literal, out);
            out.push((run + 126) as u8);
            out.push(value);
            index += run;
        } else {
            literal.push(value);
            if literal.len() == 128 {
                flush(&mut literal, out);
            }
            index += 1;
        }
    }
    flush(&mut literal, out);
}
/// Whether a PackBits stream expands to exactly one plane.
fn packed_plane(packed: &[u8]) -> bool {
    let (mut at, mut cells) = (0usize, 0usize);
    while let Some(&header) = packed.get(at) {
        let (consumed, produced) = if header < 128 {
            (1 + usize::from(header) + 1, usize::from(header) + 1)
        } else {
            (2, usize::from(header) - 126)
        };
        at += consumed;
        cells += produced;
        if at > packed.len() || cells > CELLS {
            return false;
        }
    }
    cells == CELLS
}
fn unpack(packed: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(CELLS);
    let mut at = 0usize;
    while let Some(&header) = packed.get(at) {
        if header < 128 {
            let count = usize::from(header) + 1;
            out.extend_from_slice(packed.get(at + 1..at + 1 + count)?);
            at += 1 + count;
        } else {
            let value = *packed.get(at + 1)?;
            out.resize(out.len() + usize::from(header) - 126, value);
            at += 2;
        }
        if out.len() > CELLS {
            return None;
        }
    }
    (out.len() == CELLS).then_some(out)
}

/// The coverage plane render samples: the stored plane as 0..=1, softened by `feather` (0..=100).
/// Feather 0 is the stored plane exactly; above 0 the plane takes two box passes of radius
/// `ceil(feather / 100 * 8)` cells per axis with clamped edges.
pub fn feathered(plane: &[u8], feather: f32) -> Result<Vec<f32>> {
    if plane.len() != CELLS
        || !feather.is_finite()
        || !(0.0..=project::BackgroundRemoval::MAX_FEATHER).contains(&feather)
    {
        return Err(Error::Invalid("person matte feather"));
    }
    let mut field: Vec<f32> = plane.iter().map(|v| f32::from(*v) / 255.0).collect();
    let radius =
        (feather / project::BackgroundRemoval::MAX_FEATHER * BLUR_RADIUS_AT_MAX).ceil() as usize;
    if radius > 0 {
        for _ in 0..BLUR_PASSES {
            field = blur(&field, radius, false);
            field = blur(&field, radius, true);
        }
    }
    Ok(field)
}
fn cell(value: isize) -> usize {
    value.clamp(0, SIDE as isize - 1) as usize
}
/// One clamped-edge box pass along one axis.
fn blur(source: &[f32], radius: usize, vertical: bool) -> Vec<f32> {
    let width = (2 * radius + 1) as f32;
    let mut out = Vec::with_capacity(source.len());
    for y in 0..SIDE {
        for x in 0..SIDE {
            let mut sum = 0.0;
            for k in 0..=2 * radius {
                let offset = k as isize - radius as isize;
                let (px, py) = if vertical {
                    (x, cell(y as isize + offset))
                } else {
                    (cell(x as isize + offset), y)
                };
                sum += source.get(py * SIDE + px).copied().unwrap_or_default();
            }
            out.push(sum / width);
        }
    }
    out
}

/// The affine map from a layer pixel centre `(x + 0.5, y + 0.5)` to the plane's cell coordinate,
/// `[a, b, c, d, e, f]` with `u = a px + b py + c` and `v = d px + e py + f`. `frame` maps a layer
/// point to the decoded source frame's point in pixels; the plane spans the whole frame stretched,
/// so cell centres sit at `(i + 0.5) * width / 192`.
pub fn plane_map(
    width: u32,
    height: u32,
    frame: impl Fn([f64; 2]) -> [f64; 2],
) -> Result<[f32; 6]> {
    if width == 0 || height == 0 {
        return Err(Error::Invalid("person matte frame size"));
    }
    let cells = |p: [f64; 2]| {
        let q = frame(p);
        [
            q[0] * SIDE as f64 / f64::from(width) - 0.5,
            q[1] * SIDE as f64 / f64::from(height) - 0.5,
        ]
    };
    let origin = cells([0.0, 0.0]);
    let right = cells([1.0, 0.0]);
    let down = cells([0.0, 1.0]);
    let map = [
        right[0] - origin[0],
        down[0] - origin[0],
        origin[0],
        right[1] - origin[1],
        down[1] - origin[1],
        origin[1],
    ];
    if map.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
        return Err(Error::Invalid("person matte map"));
    }
    Ok(map.map(|v| v as f32))
}

/// Bilinear coverage of the plane at layer pixel `(x, y)`; `cell` reads one plane cell. The
/// arithmetic is f32 in the order the `person_matte` shader entry uses.
fn coverage(cell_at: impl Fn(usize) -> f32, map: &[f32], x: u32, y: u32) -> f32 {
    let term = |i: usize| map.get(i).copied().unwrap_or_default();
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let u = term(0) * px + term(1) * py + term(2);
    let v = term(3) * px + term(4) * py + term(5);
    let (u0, v0) = (u.floor(), v.floor());
    let (fx, fy) = (u - u0, v - v0);
    let last = (SIDE - 1) as f32;
    let index = |value: f32| value.clamp(0.0, last) as usize;
    let (left, right) = (index(u0), index(u0 + 1.0));
    let (top, bottom) = (index(v0), index(v0 + 1.0));
    let at = |row: usize, column: usize| cell_at(row * SIDE + column);
    let upper = at(top, left) * (1.0 - fx) + at(top, right) * fx;
    let lower = at(bottom, left) * (1.0 - fx) + at(bottom, right) * fx;
    (upper * (1.0 - fy) + lower * fy).clamp(0.0, 1.0)
}

/// The application pass: the current layer times the plane's coverage. Its instance input is the
/// plane packed by [`plane_input`].
pub fn pass(map: [f32; 6]) -> EffectPass {
    EffectPass::Sample {
        entry_point: ENTRY,
        uniforms: map.map(Uniform::Float).to_vec(),
    }
}
/// The plane laid out as the pass's input image: cell `i` is the grey texel `i` in row-major
/// order, the rest transparent. A layer smaller than one plane cannot carry it.
pub fn plane_input(
    plane: &[f32],
    width: u32,
    height: u32,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    let texels = Image::bytes(width, height)? / 16;
    if plane.len() != CELLS || texels < CELLS as u64 {
        return Err(Error::Unsupported("person matte input"));
    }
    Image::from_working_pixels(
        width,
        height,
        (0..texels as usize).map(|index| {
            let v = plane.get(index).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            [v; 4]
        }),
        hooks,
    )
}
/// Uniforms are a finite map; the layer must hold one plane.
pub(crate) fn valid(values: &[f32], width: u32, height: u32) -> bool {
    u64::from(width) * u64::from(height) >= CELLS as u64 && values.iter().all(|v| v.is_finite())
}
/// CPU reference of the shader entry: the plane is read from the input image's first texels.
pub(crate) fn sample(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let width = inputs.input.width().max(1);
    let m = coverage(
        |index| {
            let index = index as u32;
            inputs.texel_input(index % width, index / width)[3]
        },
        values,
        x,
        y,
    );
    inputs.texel_current(x, y).map(|channel| channel * m)
}

/// Remove the background of one item's layer: every premultiplied pixel times the coverage of
/// `plane` (from [`feathered`]) under `map` (from [`plane_map`]). A layer that holds one plane
/// runs the `person_matte` pass on the adapter, and on the CPU reference when the adapter has no
/// such entry or answers Unsupported; both read the plane from the packed input image. A smaller
/// layer (a thumbnail) takes the same arithmetic in place.
pub fn apply(
    mut layer: Image,
    plane: &[f32],
    map: [f32; 6],
    adapter: &dyn EffectCompositor,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    check(hooks)?;
    if plane.len() != CELLS {
        return Err(Error::Invalid("person matte plane"));
    }
    let (width, height) = (layer.width(), layer.height());
    if valid(&map, width, height) {
        let input = plane_input(plane, width, height, hooks)?;
        let passes = [pass(map)];
        let inputs = || EffectInputs {
            input: &input,
            coverage: None,
            below: &layer,
        };
        if adapter.supports(ENTRY) {
            match adapter.apply(&layer, &passes, inputs(), hooks) {
                Err(Error::Unsupported(_)) => adapter.release_idle()?,
                result => return result,
            }
        }
        return CpuEffects.apply(&layer, &passes, inputs(), hooks);
    }
    for (y, row) in layer.pixels.chunks_mut(width as usize).enumerate() {
        check(hooks)?;
        for (x, pixel) in row.iter_mut().enumerate() {
            let m = coverage(
                |index| plane.get(index).copied().unwrap_or_default(),
                &map,
                x as u32,
                y as u32,
            );
            *pixel = pixel.map(|channel| channel * m);
        }
    }
    Ok(layer)
}

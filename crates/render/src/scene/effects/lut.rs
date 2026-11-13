//! Bounded display-referred cube evaluation.
use crate::{Error, ExecutionHooks, MemoryKind, Reservation, Result, check};
use std::{
    fmt,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

pub const MAX_CUBE_SOURCE: u64 = 24 << 20;
pub const MAX_CUBE_CANONICAL: u64 = 16 << 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutInvalidReason {
    Header,
    Size,
    Domain,
    Count,
    Number,
    Metadata,
    /// A declared input or output colour space other than SDR sRGB/Rec.709.
    Space,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutUnavailableReason {
    Missing,
    Changed,
    Invalid,
}
/// Source formats admitted for import. Both decode to the same canonical `.cube` text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutFormat {
    Cube,
    ThreeDl,
}
impl LutFormat {
    /// Case-insensitive `.cube` or `.3dl` extension; anything else is not a LUT source.
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("cube") {
            Some(Self::Cube)
        } else if extension.eq_ignore_ascii_case("3dl") {
            Some(Self::ThreeDl)
        } else {
            None
        }
    }
}

/// Immutable samples retain their original execution admission through every pass.
pub struct Cube3d {
    size: usize,
    min: [f64; 3],
    max: [f64; 3],
    samples: Vec<[f32; 3]>,
    identity: bool,
    _lease: Box<dyn Reservation>,
}
impl fmt::Debug for Cube3d {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cube3d")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}
impl PartialEq for Cube3d {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self.min == other.min
            && self.max == other.max
            && self.samples == other.samples
    }
}
fn invalid(line: usize, reason: LutInvalidReason) -> Error {
    Error::InvalidLut {
        line: line as u32,
        reason,
    }
}
/// Smallest common integer full scale that holds `v`.
fn bit_max(v: f32) -> f32 {
    [255.0, 1023.0, 4095.0, 16383.0, 65535.0]
        .into_iter()
        .find(|m| v <= *m)
        .unwrap_or(65535.0)
}
/// Colour-space declaration in a comment line: `INPUT_COLORSPACE: <name>` or
/// `OUTPUT_COLORSPACE: <name>` (key case-insensitive, `:` or `=` optional). Neither format
/// defines a space tag, so this is the one convention admitted; untagged tables are SDR.
/// The render pipeline carries only 8-bit SDR BT.709/sRGB pixels, so a table declaring
/// any other space refuses rather than being applied as if it were SDR.
fn space_tag(comment: &str, line: usize) -> Result<()> {
    let lower = comment.trim().to_ascii_lowercase();
    let Some(rest) = ["input_colorspace", "output_colorspace"]
        .iter()
        .find_map(|key| lower.strip_prefix(key))
    else {
        return Ok(());
    };
    let name: String = rest
        .trim_start_matches(|c: char| c == ':' || c == '=' || c.is_whitespace())
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if matches!(name.as_str(), "srgb" | "rec709" | "bt709") {
        Ok(())
    } else {
        Err(invalid(line, LutInvalidReason::Space))
    }
}
/// Samples per axis of a resampled shaped `.3dl`.
const SHAPED_SIZE: usize = 65;
/// Segment index and fraction of `t` on strictly increasing `nodes` spanning 0..=1.
fn segment(nodes: &[f64], t: f64) -> (usize, f64) {
    let last = nodes.len() - 2;
    let i = nodes
        .partition_point(|x| *x <= t)
        .saturating_sub(1)
        .min(last);
    let (Some(start), Some(end)) = (nodes.get(i), nodes.get(i + 1)) else {
        return (i, 0.);
    };
    (i, ((t - start) / (end - start)).clamp(0., 1.))
}
fn triple(text: &str, line: usize) -> Result<[f64; 3]> {
    let mut words = text.split_whitespace();
    let mut values = [0.; 3];
    for value in &mut values {
        *value = words
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .ok_or_else(|| invalid(line, LutInvalidReason::Number))?;
    }
    if words.next().is_some() {
        return Err(invalid(line, LutInvalidReason::Number));
    }
    Ok(values)
}
impl Cube3d {
    /// Strict UTF-8 3D cube, red fastest, 2..=65 samples per axis. Caller owns
    /// input admission; this parser reserves all retained sample capacity first.
    pub fn parse(bytes: &[u8], hooks: &dyn ExecutionHooks) -> Result<Self> {
        use LutInvalidReason as R;
        check(hooks)?;
        if bytes.len() as u64 > MAX_CUBE_SOURCE {
            return Err(invalid(0, R::Size));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| invalid(0, R::Header))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut size = None;
        let mut min = None;
        let mut max = None;
        let mut title = false;
        let mut samples = Vec::new();
        let mut lease = None;
        let mut header = 0usize;
        let mut body = false;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            check(hooks)?;
            if raw.len() > 1024 {
                return Err(invalid(line, R::Metadata));
            }
            if !body {
                header = header
                    .checked_add(raw.len() + 1)
                    .ok_or_else(|| invalid(line, R::Metadata))?;
                if header > 16384 {
                    return Err(invalid(line, R::Metadata));
                }
            }
            let raw = raw.trim();
            if let Some(comment) = raw.strip_prefix('#') {
                space_tag(comment, line)?;
                continue;
            }
            if raw.is_empty() {
                continue;
            }
            if raw.split_whitespace().next() == Some("TITLE") {
                if body || title {
                    return Err(invalid(line, R::Header));
                }
                let value = raw.strip_prefix("TITLE").unwrap_or_default().trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|s| s.split_once('"'))
                    .filter(|(_, tail)| tail.trim().is_empty() || tail.trim().starts_with('#'))
                    .map(|(value, _)| value)
                    .ok_or_else(|| invalid(line, R::Metadata))?;
                let _title = hooks.reserve(MemoryKind::Compositor, 16384)?;
                if captions::NfcText::from(value).as_str().len() > 256 {
                    return Err(invalid(line, R::Metadata));
                }
                title = true;
                continue;
            }
            let raw = raw.split('#').next().unwrap_or_default().trim();
            let mut words = raw.splitn(2, char::is_whitespace);
            let key = words.next().unwrap_or_default();
            let tail = words.next().unwrap_or_default().trim();
            if key
                .as_bytes()
                .first()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            {
                if body {
                    return Err(invalid(line, R::Header));
                }
                if header > 16384 {
                    return Err(invalid(line, R::Metadata));
                }
                match key {
                    "LUT_3D_SIZE" => {
                        let n = tail
                            .parse::<usize>()
                            .ok()
                            .filter(|v| (2..=65).contains(v))
                            .ok_or_else(|| invalid(line, R::Size))?;
                        if size.replace(n).is_some() {
                            return Err(invalid(line, R::Header));
                        }
                    }
                    "DOMAIN_MIN" => {
                        if min.replace(triple(tail, line)?).is_some() {
                            return Err(invalid(line, R::Header));
                        }
                    }
                    "DOMAIN_MAX" => {
                        if max.replace(triple(tail, line)?).is_some() {
                            return Err(invalid(line, R::Header));
                        }
                    }
                    "NaN" | "inf" | "Inf" | "Infinity" => return Err(invalid(line, R::Number)),
                    _ => return Err(Error::Unsupported("cube directive or shaper")),
                }
            } else {
                let n = size.ok_or_else(|| invalid(line, R::Header))?;
                let count = n.checked_pow(3).ok_or_else(|| invalid(line, R::Size))?;
                if !body {
                    if min
                        .unwrap_or([0.; 3])
                        .iter()
                        .zip(max.unwrap_or([1.; 3]))
                        .any(|(a, b)| b - *a < 1e-6)
                    {
                        return Err(invalid(line, R::Domain));
                    }
                    let bytes = count
                        .checked_mul(std::mem::size_of::<[f32; 3]>())
                        .and_then(|v| v.checked_add(std::mem::size_of::<Self>() + 64))
                        .ok_or_else(|| invalid(line, R::Size))?;
                    lease = Some(hooks.reserve(MemoryKind::Compositor, bytes as u64)?);
                    samples.try_reserve_exact(count)?;
                    // Allocators may round capacity; reserve that difference before retaining it.
                    if samples.capacity() > count {
                        return Err(Error::Budget {
                            requested: (samples.capacity() * 12) as u64,
                            limit: (count * 12) as u64,
                        });
                    }
                    body = true;
                }
                if samples.len() >= count {
                    return Err(invalid(line, R::Count));
                }
                samples.push(triple(raw, line)?.map(|v| v as f32));
            }
        }
        let size = size.ok_or_else(|| invalid(0, R::Header))?;
        if samples.len() != size.pow(3) {
            return Err(invalid(0, R::Count));
        }
        Self::assemble(
            size,
            min.unwrap_or([0.; 3]),
            max.unwrap_or([1.; 3]),
            samples,
            lease.ok_or_else(|| invalid(0, R::Count))?,
        )
    }
    /// Shared finish for every admitted format: domain check and identity detection.
    fn assemble(
        size: usize,
        min: [f64; 3],
        max: [f64; 3],
        samples: Vec<[f32; 3]>,
        lease: Box<dyn Reservation>,
    ) -> Result<Self> {
        if min.iter().zip(max).any(|(a, b)| b - *a < 1e-6) {
            return Err(invalid(0, LutInvalidReason::Domain));
        }
        let identity = min == [0.; 3]
            && max == [1.; 3]
            && samples.iter().enumerate().all(|(i, v)| {
                let expected = [i % size, (i / size) % size, i / (size * size)]
                    .map(|x| x as f32 / (size - 1) as f32);
                *v == expected
            });
        Ok(Self {
            size,
            min,
            max,
            samples,
            identity,
            _lease: lease,
        })
    }
    /// Autodesk Flame/Lustre `.3dl` (format source: OpenColorIO `FileFormat3DL.cpp`).
    /// Optional `3DMESH` and `Mesh <in> <out>` headers (`in` must give 2^in + 1 samples),
    /// one integer input mesh line of 2..=65 strictly increasing values (the shaper:
    /// the axis position of each node), then N^3 integer rows with blue changing
    /// fastest, then optional `LUT<bits>` and `gamma 1.0` footer lines. A mesh uniform to
    /// within one code value is read as is; a shaped (non-uniform) mesh is resampled
    /// trilinearly onto a uniform 65-point grid. Output depth is the `Mesh` header's or
    /// the footer's (they must agree), else the smallest integer depth holding the peak.
    /// Any other directive, footer or a gamma other than 1.0 refuses as unsupported.
    pub fn parse_3dl(bytes: &[u8], hooks: &dyn ExecutionHooks) -> Result<Self> {
        use LutInvalidReason as R;
        check(hooks)?;
        if bytes.len() as u64 > MAX_CUBE_SOURCE {
            return Err(invalid(0, R::Size));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| invalid(0, R::Header))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut size: Option<usize> = None;
        let mut domain: Option<([f64; 3], [f64; 3])> = None;
        let mut out_bits: Option<u32> = None;
        let mut in_bits: Option<u32> = None;
        let mut nodes: Option<Vec<f64>> = None;
        let mut footer_bits: Option<u32> = None;
        let mut gamma = false;
        let mut samples: Vec<[f32; 3]> = Vec::new();
        let mut lease = None;
        let mut header = 0usize;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            check(hooks)?;
            if raw.len() > 1024 {
                return Err(invalid(line, R::Metadata));
            }
            let (code, comment) = raw.split_once('#').unwrap_or((raw, ""));
            space_tag(comment, line)?;
            let raw = code.trim();
            if raw.is_empty() {
                continue;
            }
            let first = raw.split_whitespace().next().unwrap_or_default();
            if size.is_none() {
                header += raw.len() + 1;
                if header > 16384 {
                    return Err(invalid(line, R::Metadata));
                }
                if first.eq_ignore_ascii_case("3DMESH") && raw.split_whitespace().count() == 1 {
                    continue;
                }
                if first.eq_ignore_ascii_case("Mesh") {
                    let values = raw
                        .split_whitespace()
                        .skip(1)
                        .map(|v| v.parse::<u32>().ok())
                        .collect::<Option<Vec<u32>>>()
                        .ok_or_else(|| invalid(line, R::Header))?;
                    let (input, bits) = match values.as_slice() {
                        [input, bits]
                            if (0..=6u32).contains(input)
                                && (8..=16u32).contains(bits)
                                && out_bits.is_none() =>
                        {
                            (*input, *bits)
                        }
                        _ => return Err(invalid(line, R::Header)),
                    };
                    in_bits = Some(input);
                    out_bits = Some(bits);
                    continue;
                }
                if first
                    .as_bytes()
                    .first()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    return Err(Error::Unsupported("3dl directive"));
                }
                let mesh = raw
                    .split_whitespace()
                    .map(|v| v.parse::<u32>().ok())
                    .collect::<Option<Vec<u32>>>()
                    .ok_or_else(|| invalid(line, R::Number))?;
                let n = mesh.len();
                let (Some(&low), Some(&high)) = (mesh.first(), mesh.last()) else {
                    return Err(invalid(line, R::Size));
                };
                if !(2..=65).contains(&n) || mesh.windows(2).any(|w| matches!(w, [a, b] if b <= a))
                {
                    return Err(invalid(line, R::Size));
                }
                let uniform = mesh.iter().enumerate().all(|(i, m)| {
                    let expected =
                        f64::from(low) + f64::from(high - low) * i as f64 / (n - 1) as f64;
                    (f64::from(*m) - expected).abs() <= 1.0
                });
                // `Mesh <in> <out>` declares 2^in + 1 samples per axis (OpenColorIO
                // `FileFormat3DL.cpp`, Lustre example `Mesh 4 12` = 17 points).
                if in_bits.is_some_and(|b| n != (1usize << b) + 1) {
                    return Err(invalid(line, R::Header));
                }
                if !uniform {
                    let span = f64::from(high - low);
                    nodes = Some(mesh.iter().map(|m| f64::from(m - low) / span).collect());
                }
                let in_max = f64::from(bit_max(high as f32));
                domain = Some(([f64::from(low) / in_max; 3], [f64::from(high) / in_max; 3]));
                let count = n * n * n;
                // A shaped mesh is resampled onto a uniform grid; both tables are live then.
                let held = if nodes.is_some() {
                    count + SHAPED_SIZE.pow(3)
                } else {
                    count
                };
                let reserved = held
                    .checked_mul(std::mem::size_of::<[f32; 3]>())
                    .and_then(|v| v.checked_add(std::mem::size_of::<Self>() + 64))
                    .ok_or_else(|| invalid(line, R::Size))?;
                lease = Some(hooks.reserve(MemoryKind::Compositor, reserved as u64)?);
                samples.try_reserve_exact(count)?;
                if samples.capacity() > count {
                    return Err(Error::Budget {
                        requested: (samples.capacity() * 12) as u64,
                        limit: (count * 12) as u64,
                    });
                }
                size = Some(n);
                continue;
            }
            let n = size.unwrap_or_default();
            if samples.len() == n * n * n
                && first
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
            {
                // Lustre footer after the table: `LUT<bits>` and `gamma 1.0`.
                let lower = first.to_ascii_lowercase();
                let words = raw.split_whitespace().count();
                if let Some(bits) = lower
                    .strip_prefix("lut")
                    .and_then(|b| b.parse::<u32>().ok())
                    .filter(|b| (8..=16).contains(b) && words == 1 && footer_bits.is_none())
                {
                    footer_bits = Some(bits);
                } else if lower == "gamma" && words == 2 && !gamma {
                    let value = raw
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| invalid(line, R::Number))?;
                    if value != 1.0 {
                        return Err(Error::Unsupported("3dl gamma footer other than 1.0"));
                    }
                    gamma = true;
                } else {
                    return Err(Error::Unsupported("3dl footer"));
                }
                continue;
            }
            let values = raw
                .split_whitespace()
                .map(|v| v.parse::<u32>().ok())
                .collect::<Option<Vec<u32>>>()
                .ok_or_else(|| invalid(line, R::Number))?;
            let Ok([r, g, b]) = <[u32; 3]>::try_from(values.as_slice()) else {
                return Err(invalid(line, R::Number));
            };
            if samples.len() >= n * n * n {
                return Err(invalid(line, R::Count));
            }
            // Raw integer codes, exact in f32 below 2^24; normalised once the depth is known.
            samples.push([r as f32, g as f32, b as f32]);
        }
        let n = size.ok_or_else(|| invalid(0, R::Header))?;
        if samples.len() != n * n * n {
            return Err(invalid(0, R::Count));
        }
        if let Some(bits) = footer_bits {
            if out_bits.is_some_and(|b| b != bits) {
                return Err(invalid(0, R::Header));
            }
            out_bits = Some(bits);
        }
        let peak = samples.iter().flatten().fold(0f32, |a, v| a.max(*v));
        let out_max = match out_bits {
            Some(bits) => ((1u32 << bits) - 1) as f32,
            None => bit_max(peak),
        };
        if peak > out_max {
            return Err(invalid(0, R::Number));
        }
        for sample in &mut samples {
            *sample = sample.map(|v| v / out_max);
        }
        // The file lists blue fastest; the canonical table is red fastest.
        for green in 0..n {
            for red in 0..n {
                for blue in (red + 1)..n {
                    samples.swap((red * n + green) * n + blue, (blue * n + green) * n + red);
                }
            }
        }
        let (min, max) = domain.ok_or_else(|| invalid(0, R::Header))?;
        let (n, samples) = match nodes {
            Some(nodes) => (SHAPED_SIZE, Self::resample_shaped(&samples, &nodes, hooks)?),
            None => (n, samples),
        };
        Self::assemble(
            n,
            min,
            max,
            samples,
            lease.ok_or_else(|| invalid(0, R::Count))?,
        )
    }
    /// Resamples a red-fastest table whose axis nodes sit at `nodes` (0..=1, strictly
    /// increasing, the same on all three axes) onto a uniform `SHAPED_SIZE` grid by
    /// trilinear interpolation between the real nodes.
    fn resample_shaped(
        samples: &[[f32; 3]],
        nodes: &[f64],
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<[f32; 3]>> {
        let n = nodes.len();
        let m = SHAPED_SIZE;
        let axis: Vec<(usize, f64)> = (0..m)
            .map(|k| segment(nodes, k as f64 / (m - 1) as f64))
            .collect();
        let mut out = Vec::new();
        out.try_reserve_exact(m * m * m)?;
        if out.capacity() > m * m * m {
            return Err(Error::Budget {
                requested: (out.capacity() * 12) as u64,
                limit: (m * m * m * 12) as u64,
            });
        }
        for &(bi, bf) in &axis {
            check(hooks)?;
            for &(gi, gf) in &axis {
                for &(ri, rf) in &axis {
                    let mut value = [0f64; 3];
                    for (db, wb) in [(0, 1. - bf), (1, bf)] {
                        for (dg, wg) in [(0, 1. - gf), (1, gf)] {
                            for (dr, wr) in [(0, 1. - rf), (1, rf)] {
                                let weight = wb * wg * wr;
                                if weight == 0. {
                                    continue;
                                }
                                let at = ((bi + db) * n + gi + dg) * n + ri + dr;
                                let sample =
                                    samples.get(at).ok_or(Error::Invalid("shaped LUT sample"))?;
                                for (v, s) in value.iter_mut().zip(*sample) {
                                    *v += weight * f64::from(s);
                                }
                            }
                        }
                    }
                    out.push(value.map(|v| v as f32));
                }
            }
        }
        Ok(out)
    }
    /// Stable canonical text, consumed by durable import publication.
    pub fn write_cube(&self, writer: &mut dyn Write, hooks: &dyn ExecutionHooks) -> Result<()> {
        writeln!(writer, "LUT_3D_SIZE {}", self.size)?;
        for (key, values) in [("DOMAIN_MIN", self.min), ("DOMAIN_MAX", self.max)] {
            writeln!(
                writer,
                "{key} {:.17e} {:.17e} {:.17e}",
                values.first().copied().unwrap_or_default() + 0.,
                values.get(1).copied().unwrap_or_default() + 0.,
                values.get(2).copied().unwrap_or_default() + 0.
            )?;
        }
        for row in &self.samples {
            check(hooks)?;
            writeln!(
                writer,
                "{:.9e} {:.9e} {:.9e}",
                row.first().copied().unwrap_or_default() + 0.,
                row.get(1).copied().unwrap_or_default() + 0.,
                row.get(2).copied().unwrap_or_default() + 0.
            )?;
        }
        Ok(())
    }
    pub(crate) fn is_identity(&self) -> bool {
        self.identity
    }
    /// Straight encoded sRGB input/output. Edges clamp to the declared domain.
    fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut low = [0usize; 3];
        let mut t = [0.; 3];
        for (((value, minimum), maximum), (lo, weight)) in rgb
            .into_iter()
            .zip(self.min)
            .zip(self.max)
            .zip(low.iter_mut().zip(&mut t))
        {
            let x = ((f64::from(value) - minimum) / (maximum - minimum)).clamp(0., 1.)
                * (self.size - 1) as f64;
            *lo = (x.floor() as usize).min(self.size - 2);
            *weight = x - *lo as f64;
        }
        let mut out = [0f64; 3];
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    let offsets = [r, g, b];
                    let mut weight = 1.;
                    let mut coords = [0; 3];
                    for (((offset, lo), fraction), coord) in
                        offsets.into_iter().zip(low).zip(t).zip(&mut coords)
                    {
                        *coord = lo + offset;
                        weight *= if offset == 0 { 1. - fraction } else { fraction };
                    }
                    let [r, g, b] = coords;
                    if let Some(row) = self.samples.get((b * self.size + g) * self.size + r) {
                        for (v, s) in out.iter_mut().zip(row) {
                            *v += f64::from(*s) * weight;
                        }
                    }
                }
            }
        }
        out.map(|v| v.clamp(0., 1.) as f32)
    }
    pub(crate) fn pixel(&self, pixel: [f32; 4], strength: f32) -> [f32; 4] {
        if self.identity || strength == 0. {
            return pixel;
        }
        let [r, g, b, a] = pixel;
        if a == 0. {
            return [0.; 4];
        }
        let dry = [r / a, g / a, b / a];
        let encoded = dry.map(|v| crate::image::oetf(v, 13));
        let wet = self.sample(encoded).map(|v| crate::image::eotf(v, 13));
        let mut output = [0.; 4];
        for ((out, dry), wet) in output.iter_mut().take(3).zip(dry).zip(wet) {
            *out = ((1. - strength) * dry + strength * wet).clamp(0., 1.) * a;
        }
        if let Some(alpha) = output.get_mut(3) {
            *alpha = a;
        }
        output
    }
}

/// Read a bounded regular file without following a final symlink. The input lease
/// remains live until parsing is complete, including sample allocation overlap.
pub fn read_lut(
    path: &Path,
    format: LutFormat,
    limit: u64,
    hooks: &dyn ExecutionHooks,
) -> Result<(Cube3d, media::ContentHash)> {
    let limit = limit.min(MAX_CUBE_SOURCE);
    for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid(0, LutInvalidReason::Metadata));
        }
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        return Err(invalid(0, LutInvalidReason::Size));
    }
    let capacity = meta
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid(0, LutInvalidReason::Size))?;
    let _input = hooks.reserve(MemoryKind::Compositor, capacity + 65536)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity as usize)?;
    if bytes.capacity() as u64 > capacity {
        return Err(Error::Budget {
            requested: bytes.capacity() as u64,
            limit: capacity,
        });
    }
    let mut file = std::fs::File::open(path)?.take(capacity);
    let mut chunk = [0u8; 65536];
    loop {
        check(hooks)?;
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(
            chunk
                .get(..n)
                .ok_or_else(|| invalid(0, LutInvalidReason::Size))?,
        );
    }
    if bytes.len() as u64 != meta.len() {
        return Err(invalid(0, LutInvalidReason::Size));
    }
    let hash = media::ContentHash::from_bytes(&bytes);
    let cube = match format {
        LutFormat::Cube => Cube3d::parse(&bytes, hooks)?,
        LutFormat::ThreeDl => Cube3d::parse_3dl(&bytes, hooks)?,
    };
    Ok((cube, hash))
}

/// Closed per-frame resources. Every table belongs to the exact hooks supplied to
/// this resolution; no foreign-policy cache entry can supply an uncharged Arc.
// ponytail: zero idle tables; add a bounded, hook-owned cache after measured need.
#[derive(Default)]
pub struct LutResources {
    tables: std::collections::BTreeMap<project::LutId, Arc<Cube3d>>,
    _lease: Option<Box<dyn Reservation>>,
}
impl LutResources {
    pub fn resolve<'a>(
        project: &project::Project,
        effects: impl Iterator<Item = &'a project::EffectInstance>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        let mut resources = Self::default();
        for effect in effects.filter(|e| e.enabled && e.kind == "color_grade") {
            if effect
                .value("lut_strength")
                .and_then(project::EffectValue::scalar)
                == Some(0.)
            {
                continue;
            }
            if let Some(project::EffectValue::Lut(Some(id))) = effect.value("lut") {
                if resources.tables.contains_key(id) {
                    continue;
                }
                if resources._lease.is_none() {
                    resources._lease = Some(hooks.reserve(MemoryKind::Compositor, 16384)?);
                }
                let record = project.data().luts.iter().find(|r| r.id == *id).ok_or(
                    Error::LutUnavailable {
                        id: *id,
                        reason: LutUnavailableReason::Missing,
                    },
                )?;
                let path = record.directory.join("canonical.cube");
                let (cube, hash) = read_lut(&path, LutFormat::Cube, MAX_CUBE_CANONICAL, hooks)
                    .map_err(|error| match error {
                        Error::Io(ref io) if io.kind() == std::io::ErrorKind::NotFound => {
                            Error::LutUnavailable {
                                id: *id,
                                reason: LutUnavailableReason::Missing,
                            }
                        }
                        Error::InvalidLut { .. } | Error::Unsupported(_) => Error::LutUnavailable {
                            id: *id,
                            reason: LutUnavailableReason::Invalid,
                        },
                        other => other,
                    })?;
                if project::LutId::new(hash) != *id {
                    return Err(Error::LutUnavailable {
                        id: *id,
                        reason: LutUnavailableReason::Changed,
                    });
                }
                resources.tables.insert(*id, Arc::new(cube));
            }
        }
        Ok(resources)
    }
    pub(crate) fn get(&self, id: project::LutId) -> Result<Arc<Cube3d>> {
        self.tables.get(&id).cloned().ok_or(Error::LutUnavailable {
            id,
            reason: LutUnavailableReason::Missing,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    #[derive(Default)]
    struct Hooks {
        bytes: Arc<AtomicU64>,
        cancel: AtomicBool,
    }
    struct Lease(Arc<AtomicU64>, u64);
    impl Reservation for Lease {}
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_sub(self.1, Ordering::SeqCst);
        }
    }
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::SeqCst)
        }
        fn reserve(&self, _: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
            self.bytes.fetch_add(bytes, Ordering::SeqCst);
            Ok(Box::new(Lease(self.bytes.clone(), bytes)))
        }
    }
    fn table(n: usize, identity: bool) -> String {
        let mut source = format!("LUT_3D_SIZE {n}\n");
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let [r, g, b] = [r, g, b].map(|x| x as f64 / (n - 1) as f64);
                    let [x, y, z] = if identity {
                        [r, g, b]
                    } else {
                        [r * g, g * b, b * r]
                    };
                    source.push_str(&format!("{x:.9} {y:.9} {z:.9}\n"));
                }
            }
        }
        source
    }
    #[test]
    fn cube_interpolation_matches_independent_oracle() {
        let hooks = Hooks::default();
        for n in [2, 17, 33, 65] {
            let cube = Cube3d::parse(table(n, false).as_bytes(), &hooks).unwrap();
            for [r, g, b] in [
                [0., 0., 0.],
                [1., 1., 1.],
                [0.23, 0.71, 0.49],
                [1., 0., 0.5],
            ] {
                let expected = [r * g, g * b, b * r];
                for (got, expected) in cube.sample([r, g, b]).into_iter().zip(expected) {
                    assert!((got - expected).abs() < 3e-6, "N={n}");
                }
            }
            // Independent f64 transfer and wet/dry oracle, not production helpers.
            let encode = |v: f64| {
                if v <= 0.0031308 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1. / 2.4) - 0.055
                }
            };
            let decode = |v: f64| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            for alpha in [0., 0.25, 1.] {
                for strength in [0., 0.5, 1.] {
                    let dry = [0.19f64, 0.42, 0.73];
                    let [r, g, b] = dry.map(encode);
                    let expected = [r * g, g * b, b * r].map(decode);
                    let pixel = [
                        (dry[0] * alpha) as f32,
                        (dry[1] * alpha) as f32,
                        (dry[2] * alpha) as f32,
                        alpha as f32,
                    ];
                    let got = cube.pixel(pixel, strength as f32);
                    assert_eq!(got[3], pixel[3]);
                    if strength == 0. {
                        assert_eq!(got, pixel);
                    }
                    for i in 0..3 {
                        assert!(
                            (f64::from(got[i])
                                - ((1. - strength) * dry[i] + strength * expected[i]) * alpha)
                                .abs()
                                < 3e-6
                        );
                    }
                }
            }
            let mut canonical = Vec::new();
            cube.write_cube(&mut canonical, &hooks).unwrap();
            assert!(canonical.len() as u64 <= MAX_CUBE_CANONICAL);
            let second = Cube3d::parse(&canonical, &hooks).unwrap();
            let mut again = Vec::new();
            second.write_cube(&mut again, &hooks).unwrap();
            assert_eq!(canonical, again);
        }
        let permutation = Cube3d::parse(
            b"LUT_3D_SIZE 2\n0 0 0\n0 1 0\n0 0 1\n0 1 1\n1 0 0\n1 1 0\n1 0 1\n1 1 1\n",
            &hooks,
        )
        .unwrap();
        for (actual, expected) in permutation
            .sample([0.23, 0.71, 0.49])
            .into_iter()
            .zip([0.49, 0.23, 0.71])
        {
            assert!((actual - expected).abs() < 3e-6);
        }
        // Eight literal, asymmetric corners with independent f64 weights. This
        // catches axis swaps even when a separable or multi-affine ramp agrees.
        let asymmetric = Cube3d::parse(
            b"LUT_3D_SIZE 2\nDOMAIN_MIN 0.25 0.1 0.2\nDOMAIN_MAX 0.75 0.9 0.8\n0.04 0.2 0.8\n0.7 0.1 0.3\n0.2 0.9 0.1\n0.8 0.6 0.4\n0.3 0.5 0.9\n0.1 0.7 0.2\n0.9 0.3 0.6\n0.6 0.4 0.7\n",
            &hooks,
        ).unwrap();
        let [r, g, b] = [0.17f64, 0.39, 0.62];
        let weights = [
            (1. - r) * (1. - g) * (1. - b),
            r * (1. - g) * (1. - b),
            (1. - r) * g * (1. - b),
            r * g * (1. - b),
            (1. - r) * (1. - g) * b,
            r * (1. - g) * b,
            (1. - r) * g * b,
            r * g * b,
        ];
        let corners = [
            [0.04, 0.2, 0.8],
            [0.7, 0.1, 0.3],
            [0.2, 0.9, 0.1],
            [0.8, 0.6, 0.4],
            [0.3, 0.5, 0.9],
            [0.1, 0.7, 0.2],
            [0.9, 0.3, 0.6],
            [0.6, 0.4, 0.7],
        ];
        let actual = asymmetric.sample([
            (0.25 + r * 0.5) as f32,
            (0.1 + g * 0.8) as f32,
            (0.2 + b * 0.6) as f32,
        ]);
        for channel in 0..3 {
            let expected: f64 = weights
                .iter()
                .zip(corners)
                .map(|(w, c)| w * c[channel])
                .sum();
            assert!((f64::from(actual[channel]) - expected).abs() < 3e-6);
        }
        drop((permutation, asymmetric));
        assert_eq!(hooks.bytes.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn cube_refuses_hostile_inputs() {
        let hooks = Hooks::default();
        let valid = table(2, false);
        for source in [
            String::new(),
            valid.replacen("2", "1", 1),
            valid.replacen("2", "66", 1),
            valid.replacen("2", "18446744073709551615", 1),
            valid.lines().take(8).collect::<Vec<_>>().join("\n"),
            format!("{valid}0 0 0\n"),
            valid.replacen("0.000000000", "-0.1", 1),
            valid.replacen("0.000000000", "1.01", 1),
            valid.replacen("0.000000000", "NaN", 1),
            valid.replacen("0.000000000", "inf", 1),
            valid.replacen("0.000000000", "1e9999", 1),
            format!("TITLE\"not a directive\"\n{valid}"),
            format!("TITLE \"a\"\nTITLE \"b\"\n{valid}"),
            format!("{valid}TITLE \"late\"\n"),
            format!("DOMAIN_MIN 0 0 0\nDOMAIN_MIN 0 0 0\n{valid}"),
            format!("DOMAIN_MIN 0 0 0\nDOMAIN_MAX 0.00000001 1 1\n{valid}"),
            format!("{valid}DOMAIN_MIN 0 0 0\n"),
            format!("LUT_1D_SIZE 2\n{valid}"),
            format!("TITLE \"{}\"\n{valid}", "x".repeat(257)),
        ] {
            assert!(Cube3d::parse(source.as_bytes(), &hooks).is_err());
            assert_eq!(hooks.bytes.load(Ordering::SeqCst), 0);
        }
        assert!(Cube3d::parse(b"\xff", &hooks).is_err());
        hooks.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(
            Cube3d::parse(valid.as_bytes(), &hooks),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn cube_identity_domains_and_input_lease() {
        let hooks = Hooks::default();
        let valid = table(17, true);
        let cube = Cube3d::parse(
            format!("\u{feff}TITLE \"My # title\" # \"comment\"\r\n{valid}").as_bytes(),
            &hooks,
        )
        .unwrap();
        assert!(cube.is_identity());
        assert_eq!(cube.pixel([0.1, 0.2, 0.3, 0.4], 1.), [0.1, 0.2, 0.3, 0.4]);
        drop(cube);
        for (title, accepted) in [
            ("é".repeat(128), true),
            ("e\u{301}".repeat(128), true),
            ("e\u{301}".repeat(129), false),
        ] {
            assert_eq!(
                Cube3d::parse(format!("TITLE \"{title}\"\n{valid}").as_bytes(), &hooks).is_ok(),
                accepted,
                "TITLE bound applies after NFC"
            );
        }
        let domain = Cube3d::parse(
            format!(
                "DOMAIN_MIN 0.25 0.25 0.25\nDOMAIN_MAX 0.75 0.75 0.75\n{}",
                table(2, true)
            )
            .as_bytes(),
            &hooks,
        )
        .unwrap();
        assert!(!domain.is_identity());
        assert_eq!(domain.sample([0.25, 0.75, 0.5]), [0., 1., 0.5]);
        drop(domain);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("look.cube");
        std::fs::write(&path, &valid).unwrap();
        let (cube, hash) = read_lut(&path, LutFormat::Cube, MAX_CUBE_SOURCE, &hooks).unwrap();
        assert_eq!(hash, media::ContentHash::from_bytes(valid.as_bytes()));
        assert!(hooks.bytes.load(Ordering::SeqCst) > 17 * 17 * 17 * 12);
        drop(cube);
        assert_eq!(hooks.bytes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn three_dl_uses_blue_fastest_rows_and_refuses_hostile_inputs() {
        let hooks = Hooks::default();
        // Two-sample LUT: 10-bit mesh 0..1023, blue changes fastest in the file.
        // Equal to the .cube table below, which is red fastest.
        let body = "0 0 0\n0 1023 0\n1023 0 0\n1023 1023 0\n0 0 1023\n0 1023 1023\n1023 0 1023\n1023 1023 1023\n";
        let valid = format!("3DMESH\n0 1023\n{body}");
        let cube = Cube3d::parse(
            b"LUT_3D_SIZE 2\n0 0 0\n0 0 1\n1 0 0\n1 0 1\n0 1 0\n0 1 1\n1 1 0\n1 1 1\n",
            &hooks,
        )
        .unwrap();
        let (mut expected, mut actual) = (Vec::new(), Vec::new());
        cube.write_cube(&mut expected, &hooks).unwrap();
        Cube3d::parse_3dl(valid.as_bytes(), &hooks)
            .unwrap()
            .write_cube(&mut actual, &hooks)
            .unwrap();
        assert_eq!(actual, expected);
        drop(cube);
        let sixty_six = (0..66).map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
        for source in [
            String::new(),
            format!("3DMESH\n{body}"),
            format!("3DMESH\n0 1023 2047\n{body}"),
            format!("3DMESH\n1023 0\n{body}"),
            format!("3DMESH\n0 1 1023\n{body}"),
            format!("3DMESH\n{sixty_six}\n{body}"),
            format!("LUT8\n{valid}"),
            format!(
                "Mesh 10 10\n{}",
                valid.replace("1023 1023 1023", "1023 1023 1024")
            ),
            format!("Mesh 10 9\n{valid}"),
            format!(
                "3DMESH\n0 1023\n{}",
                body.replace("1023 1023 1023", "-1 0 0")
            ),
            format!("3DMESH\n0 1023\n{body}0 0 0\n"),
            format!(
                "3DMESH\n0 1023\n{}",
                body.lines().take(7).collect::<Vec<_>>().join("\n")
            ),
            format!(
                "3DMESH\n0 1023\n{}",
                body.replace("1023 1023 1023", "1023 1023")
            ),
        ] {
            assert!(Cube3d::parse_3dl(source.as_bytes(), &hooks).is_err());
            assert_eq!(hooks.bytes.load(Ordering::SeqCst), 0);
        }
        hooks.cancel.store(true, Ordering::SeqCst);
        assert!(matches!(
            Cube3d::parse_3dl(valid.as_bytes(), &hooks),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn three_dl_shaper_footer_and_space_variants() {
        let hooks = Hooks::default();
        let body = "0 0 0
0 1023 0
1023 0 0
1023 1023 0
0 0 1023
0 1023 1023
1023 0 1023
1023 1023 1023
";
        let valid = format!(
            "3DMESH
0 1023
{body}"
        );
        // Shaped mesh: nodes 0, 100, 1023 hold the identity at those positions, so the
        // uniform resample reproduces the identity everywhere.
        let nodes = [0u32, 100, 1023];
        let mut shaped = String::from(
            "3DMESH
Mesh 1 10
0 100 1023
",
        );
        for r in nodes {
            for g in nodes {
                for b in nodes {
                    shaped.push_str(&format!(
                        "{r} {g} {b}
"
                    ));
                }
            }
        }
        // Mesh 1 means 3 samples per axis.
        let cube = Cube3d::parse_3dl(shaped.as_bytes(), &hooks).unwrap();
        assert_eq!(cube.size, SHAPED_SIZE);
        for probe in [[0.05, 0.5, 0.97], [0.1, 0.2, 0.3], [1., 0., 0.4]] {
            let out = cube.sample(probe);
            for (a, b) in out.iter().zip(probe) {
                assert!((a - b).abs() < 1e-4, "{out:?} vs {probe:?}");
            }
        }
        drop(cube);
        // Footer and tag variants that load.
        for source in [
            format!(
                "{valid}LUT10
gamma 1.0
"
            ),
            format!(
                "Mesh 0 10
0 1023
{body}LUT10
"
            ),
            format!(
                "# INPUT_COLORSPACE: Rec.709
# output_colorspace = sRGB
{valid}"
            ),
            format!(
                "{valid}# INPUT_COLORSPACE: BT.709
"
            ),
        ] {
            Cube3d::parse_3dl(source.as_bytes(), &hooks).unwrap();
        }
        // Variants that refuse: footer conflicts, gamma, unknown footer, wrong Mesh size,
        // and a declared space the SDR pipeline does not carry.
        for source in [
            format!(
                "Mesh 0 10
0 1023
{body}LUT12
"
            ),
            format!(
                "{valid}gamma 2.2
"
            ),
            format!(
                "{valid}LUT10
LUT10
"
            ),
            format!(
                "{valid}shaper 1
"
            ),
            format!(
                "Mesh 2 10
0 1023
{body}"
            ),
            format!(
                "# INPUT_COLORSPACE: Rec2020
{valid}"
            ),
            format!(
                "{valid}# OUTPUT_COLORSPACE: ACEScg
"
            ),
        ] {
            assert!(Cube3d::parse_3dl(source.as_bytes(), &hooks).is_err());
            assert_eq!(hooks.bytes.load(Ordering::SeqCst), 0);
        }
        let cube_source = |tag: &str| {
            format!(
                "{tag}LUT_3D_SIZE 2
0 0 0
1 0 0
0 1 0
1 1 0
0 0 1
1 0 1
0 1 1
1 1 1
"
            )
        };
        Cube3d::parse(
            cube_source(
                "# INPUT_COLORSPACE: sRGB
",
            )
            .as_bytes(),
            &hooks,
        )
        .unwrap();
        assert!(matches!(
            Cube3d::parse(
                cube_source(
                    "# OUTPUT_COLORSPACE: P3-D65
"
                )
                .as_bytes(),
                &hooks
            ),
            Err(Error::InvalidLut {
                reason: LutInvalidReason::Space,
                ..
            })
        ));
    }

    #[test]
    fn cube_budget_denial_and_growing_file_refuse_before_retention() {
        struct Denied;
        impl ExecutionHooks for Denied {
            fn cancelled(&self) -> bool {
                false
            }
            fn reserve(&self, _: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
                Err(Error::Budget {
                    requested: bytes,
                    limit: 0,
                })
            }
        }
        assert!(matches!(
            Cube3d::parse(table(65, false).as_bytes(), &Denied),
            Err(Error::Budget { .. })
        ));
        struct Grow {
            path: std::path::PathBuf,
            once: AtomicBool,
        }
        impl ExecutionHooks for Grow {
            fn cancelled(&self) -> bool {
                false
            }
            fn reserve(&self, _: MemoryKind, _: u64) -> Result<Box<dyn Reservation>> {
                if !self.once.swap(true, Ordering::SeqCst) {
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(&self.path)
                        .unwrap()
                        .write_all(b"0 0 0\n")
                        .unwrap();
                }
                Ok(Box::new(Lease(Arc::new(AtomicU64::new(0)), 0)))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.cube");
        std::fs::write(&path, table(2, false)).unwrap();
        let hooks = Grow {
            path: path.clone(),
            once: AtomicBool::new(false),
        };
        assert!(matches!(
            read_lut(&path, LutFormat::Cube, MAX_CUBE_SOURCE, &hooks),
            Err(Error::InvalidLut {
                reason: LutInvalidReason::Size,
                ..
            })
        ));
    }

    #[test]
    fn lut_pass_retains_its_lease_across_device_runs_and_only_unsupported_retries() {
        use crate::effects::{
            CpuEffects, EffectCompositor, EffectInputs, EffectPass, apply_effect, grade_passes,
        };
        struct Device {
            fail: u8,
            releases: std::sync::atomic::AtomicUsize,
        }
        impl EffectCompositor for Device {
            fn frame<'a>(
                &'a self,
                _: &'a dyn ExecutionHooks,
            ) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
                Ok(None)
            }
            fn release_idle(&self) -> Result<()> {
                self.releases.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            fn supports(&self, entry: &str) -> bool {
                entry == "grade"
            }
            fn apply(
                &self,
                image: &crate::Image,
                passes: &[EffectPass],
                inputs: EffectInputs<'_>,
                hooks: &dyn ExecutionHooks,
            ) -> Result<crate::Image> {
                assert_eq!(inputs.input.pixels()[0], [0.1, 0.2, 0.3, 0.5]);
                match self.fail {
                    1 => Err(Error::Unsupported("test device")),
                    2 => Err(Error::Invalid("test device")),
                    3 => Err(Error::Cancelled),
                    _ => CpuEffects.apply(image, passes, inputs, hooks),
                }
            }
        }
        let hooks = Hooks::default();
        let table = Arc::new(Cube3d::parse(table(2, false).as_bytes(), &hooks).unwrap());
        let table_bytes = hooks.bytes.load(Ordering::SeqCst);
        let mut passes = grade_passes(10., 0., 0., 0.).unwrap();
        passes.push(EffectPass::Lut3d {
            table: table.clone(),
            strength: 0.5,
        });
        passes.extend(grade_passes(-5., 0., 0., 0.).unwrap());
        drop(table);
        assert_eq!(hooks.bytes.load(Ordering::SeqCst), table_bytes);
        let below = crate::Image::new(2, 2, [0.2, 0.1, 0.3, 0.5], &hooks).unwrap();
        let expected_below = below.pixels().to_vec();
        let source = || crate::Image::new(2, 2, [0.1, 0.2, 0.3, 0.5], &hooks).unwrap();
        let expected = apply_effect(source(), &passes, &below, &CpuEffects, &hooks).unwrap();
        for fail in [0, 1, 2, 3] {
            let device = Device {
                fail,
                releases: std::sync::atomic::AtomicUsize::new(0),
            };
            let result = apply_effect(source(), &passes, &below, &device, &hooks);
            match fail {
                0 | 1 => {
                    assert_eq!(result.unwrap().pixels(), expected.pixels());
                    assert!(device.releases.load(Ordering::SeqCst) > 0);
                }
                2 => assert!(matches!(result, Err(Error::Invalid("test device")))),
                _ => assert!(matches!(result, Err(Error::Cancelled))),
            }
            assert_eq!(below.pixels(), expected_below);
        }
        drop((expected, below));
        assert_eq!(hooks.bytes.load(Ordering::SeqCst), table_bytes);
        drop(passes);
        assert_eq!(hooks.bytes.load(Ordering::SeqCst), 0);
    }
}

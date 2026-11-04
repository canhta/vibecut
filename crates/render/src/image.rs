use crate::{Error, ExecutionHooks, MemoryKind, Reservation, Result, vector};
use std::sync::OnceLock;
/// Row-major, RGBA, premultiplied alpha, linear light, finite f32 in 0..=1.
/// RGB never exceeds alpha. Zero size is a typed error, never an empty fallback.
pub struct Image {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<[f32; 4]>,
    _lease: Box<dyn Reservation>,
}
impl Image {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }
    pub fn bytes(width: u32, height: u32) -> Result<u64> {
        let area = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || area > 1 << 26 {
            return Err(Error::Invalid("image dimensions"));
        }
        Ok(area * 16)
    }
    pub fn new(
        width: u32,
        height: u32,
        color: [f32; 4],
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        if color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || color
                .iter()
                .take(3)
                .any(|v| *v > color.get(3).copied().unwrap_or(0.0))
        {
            return Err(Error::Invalid("premultiplied color"));
        }
        let bytes = Self::bytes(width, height)?;
        let lease = hooks.reserve(MemoryKind::Compositor, bytes)?;
        Ok(Self {
            width,
            height,
            pixels: vector((bytes / 16) as usize, color)?,
            _lease: lease,
        })
    }
    pub fn from_rgba8(
        width: u32,
        height: u32,
        bytes: &[u8],
        premultiplied: bool,
        transfer: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        if !matches!(transfer, 1 | 13 | 8) {
            return Err(Error::Unsupported("transfer function"));
        }
        let size = Self::bytes(width, height)?;
        if bytes.len() as u64 != size / 4 {
            return Err(Error::Invalid("RGBA input size/premultiplication"));
        }
        Self::from_rgba8_strided(
            width,
            height,
            bytes,
            width as usize * 4,
            premultiplied,
            transfer,
            hooks,
        )
    }
    pub(crate) fn from_rgba8_strided(
        width: u32,
        height: u32,
        bytes: &[u8],
        stride: usize,
        premultiplied: bool,
        transfer: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self> {
        Self::bytes(width, height)?;
        if !matches!(transfer, 1 | 13 | 8) {
            return Err(Error::Unsupported("transfer function"));
        }
        let row = width as usize * 4;
        let required = stride
            .checked_mul(height as usize - 1)
            .and_then(|n| n.checked_add(row))
            .ok_or(Error::Invalid("RGBA source stride"))?;
        if stride < row || bytes.len() < required {
            return Err(Error::Invalid("RGBA source stride"));
        }
        let rows = || {
            bytes
                .chunks(stride)
                .take(height as usize)
                .map(|s| s.get(..row).unwrap_or(&[]))
        };
        if premultiplied
            && rows().any(|row| {
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|&[r, g, b, a]| r > a || g > a || b > a)
            })
        {
            return Err(Error::Invalid("RGBA input size/premultiplication"));
        }
        let mut image = Self::new(width, height, [0.0; 4], hooks)?;
        let lut = decode_lut(transfer);
        for (destination, source) in image.pixels.chunks_mut(width as usize).zip(rows()) {
            crate::check(hooks)?;
            if !premultiplied {
                for (dst, &[r, g, b, a]) in destination.iter_mut().zip(source.as_chunks::<4>().0) {
                    let alpha = f32::from(a) / 255.0;
                    let channel = |v: u8| lut.get(usize::from(v)).copied().unwrap_or(0.0) * alpha;
                    *dst = [channel(r), channel(g), channel(b), alpha];
                }
            } else {
                for (dst, &[r, g, b, a]) in destination.iter_mut().zip(source.as_chunks::<4>().0) {
                    let alpha = f32::from(a) / 255.0;
                    let channel = |v: u8| {
                        let value = if alpha > 0.0 {
                            (f32::from(v) / alpha).round().clamp(0.0, 255.0) as usize
                        } else {
                            usize::from(v)
                        };
                        lut.get(value).copied().unwrap_or(0.0) * alpha
                    };
                    *dst = [channel(r), channel(g), channel(b), alpha];
                }
            }
        }
        Ok(image)
    }
    /// Straight transfer-encoded RGBA8 at the image/FFmpeg boundary.
    pub fn rgba8(&self, transfer: u32, hooks: &dyn ExecutionHooks) -> Result<Rgba8> {
        self.pack(transfer, false, hooks)
    }
    /// Opaque MP4 boundary: composite residual transparency over black in linear
    /// light before encoding. PNG/other alpha-aware consumers use rgba8 instead.
    pub fn video_rgba8(&self, transfer: u32, hooks: &dyn ExecutionHooks) -> Result<Rgba8> {
        self.pack(transfer, true, hooks)
    }
    fn pack(&self, transfer: u32, opaque: bool, hooks: &dyn ExecutionHooks) -> Result<Rgba8> {
        crate::check(hooks)?;
        let lut = encode_lut(transfer)?;
        let bytes = self.pixels.len() * 4;
        let lease = hooks.reserve(MemoryKind::Export, bytes as u64)?;
        let mut output = vector(bytes, 0u8)?;
        if opaque {
            for (&[r, g, b, _], dst) in self.pixels.iter().zip(output.as_chunks_mut::<4>().0) {
                let channel = |v: f32| {
                    lut.get((v.clamp(0.0, 1.0) * 65535.0).round() as usize)
                        .copied()
                        .unwrap_or(0)
                };
                *dst = [channel(r), channel(g), channel(b), 255];
            }
        } else {
            for (&[r, g, b, a], dst) in self.pixels.iter().zip(output.as_chunks_mut::<4>().0) {
                let channel = |v: f32| {
                    let straight = if a > 0.0 {
                        (v / a).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    lut.get((straight * 65535.0).round() as usize)
                        .copied()
                        .unwrap_or(0)
                };
                *dst = [
                    channel(r),
                    channel(g),
                    channel(b),
                    (a * 255.0).round() as u8,
                ];
            }
        }
        Ok(Rgba8 {
            bytes: output,
            _lease: lease,
        })
    }
    pub(crate) fn at(&self, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return [0.0; 4];
        }
        self.pixels
            .get(y as usize * self.width as usize + x as usize)
            .copied()
            .unwrap_or([0.0; 4])
    }
    pub(crate) fn half(&self, hooks: &dyn ExecutionHooks) -> Result<Self> {
        let w = self.width.div_ceil(2);
        let h = self.height.div_ceil(2);
        let mut out = Self::new(w, h, [0.0; 4], hooks)?;
        for (i, p) in out.pixels.iter_mut().enumerate() {
            let x = (i % w as usize) * 2;
            let y = (i / w as usize) * 2;
            let mut count = 0f32;
            for yy in y..(y + 2).min(self.height as usize) {
                for xx in x..(x + 2).min(self.width as usize) {
                    if let Some(source) = self.pixels.get(yy * self.width as usize + xx) {
                        for (d, s) in p.iter_mut().zip(source) {
                            *d += s;
                        }
                        count += 1.0;
                    }
                }
            }
            for value in p {
                *value /= count;
            }
        }
        Ok(out)
    }
}
pub struct Rgba8 {
    pub bytes: Vec<u8>,
    _lease: Box<dyn Reservation>,
}
pub(crate) fn color(bytes: [u8; 4]) -> [f32; 4] {
    let a = f32::from(bytes.get(3).copied().unwrap_or(0)) / 255.0;
    std::array::from_fn(|k| {
        if k == 3 {
            a
        } else {
            decode_lut(13)
                .get(usize::from(bytes.get(k).copied().unwrap_or(0)))
                .copied()
                .unwrap_or(0.0)
                * a
        }
    })
}
fn eotf(x: f32, transfer: u32) -> f32 {
    match transfer {
        8 => x,
        13 => {
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        }
        _ => {
            if x < 0.081 {
                x / 4.5
            } else {
                ((x + 0.099) / 1.099).powf(1.0 / 0.45)
            }
        }
    }
}
fn oetf(x: f32, transfer: u32) -> f32 {
    match transfer {
        8 => x,
        13 => {
            if x <= 0.0031308 {
                x * 12.92
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            }
        }
        _ => {
            if x < 0.018 {
                x * 4.5
            } else {
                1.099 * x.powf(0.45) - 0.099
            }
        }
    }
}
fn decode_lut(transfer: u32) -> &'static [f32; 256] {
    static SRGB: OnceLock<[f32; 256]> = OnceLock::new();
    static VIDEO: OnceLock<[f32; 256]> = OnceLock::new();
    static LINEAR: OnceLock<[f32; 256]> = OnceLock::new();
    let slot = match transfer {
        13 => &SRGB,
        8 => &LINEAR,
        _ => &VIDEO,
    };
    slot.get_or_init(|| std::array::from_fn(|i| eotf(i as f32 / 255.0, transfer)))
}
fn encode_lut(transfer: u32) -> Result<&'static [u8]> {
    static SRGB: OnceLock<Vec<u8>> = OnceLock::new();
    static VIDEO: OnceLock<Vec<u8>> = OnceLock::new();
    static LINEAR: OnceLock<Vec<u8>> = OnceLock::new();
    let slot = match transfer {
        13 => &SRGB,
        8 => &LINEAR,
        1 => &VIDEO,
        _ => return Err(Error::Unsupported("output transfer")),
    };
    Ok(slot.get_or_init(|| {
        (0..65536)
            .map(|i| {
                (oetf(i as f32 / 65535.0, transfer) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect()
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    struct Hooks;
    struct Lease;
    impl Reservation for Lease {}
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn reserve(&self, _: MemoryKind, _: u64) -> Result<Box<dyn Reservation>> {
            Ok(Box::new(Lease))
        }
    }
    #[test]
    fn strided_ingress_matches_public_tight_ingress_and_refuses_hostile_rows() {
        let tight = [
            0, 0, 0, 0, 16, 32, 64, 128, 80, 60, 40, 100, 255, 255, 255, 255,
        ];
        // Padding is deliberately invalid premultiplied color; it is not a pixel.
        let padded = [
            0, 0, 0, 0, 16, 32, 64, 128, 255, 255, 255, 0, 80, 60, 40, 100, 255, 255, 255, 255,
        ];
        for premultiplied in [false, true] {
            for transfer in [1, 8, 13] {
                let reference =
                    Image::from_rgba8(2, 2, &tight, premultiplied, transfer, &Hooks).unwrap();
                let actual =
                    Image::from_rgba8_strided(2, 2, &padded, 12, premultiplied, transfer, &Hooks)
                        .unwrap();
                assert_eq!(reference.pixels(), actual.pixels());
                assert_eq!(
                    reference.rgba8(transfer, &Hooks).unwrap().bytes,
                    actual.rgba8(transfer, &Hooks).unwrap().bytes
                );
                assert_eq!(
                    reference.video_rgba8(transfer, &Hooks).unwrap().bytes,
                    actual.video_rgba8(transfer, &Hooks).unwrap().bytes
                );
            }
        }
        for (width, height, stride, bytes) in [
            (2, 2, 7, padded.as_slice()),
            (2, 2, usize::MAX, &padded),
            (2, 2, 12, &padded[..19]),
            (0, 2, 12, &padded),
            (2, 0, 12, &padded),
        ] {
            assert!(
                Image::from_rgba8_strided(width, height, bytes, stride, false, 1, &Hooks).is_err()
            );
        }
        assert!(Image::from_rgba8_strided(1, 1, &[255, 0, 0, 128], 4, true, 1, &Hooks).is_err());
    }
}

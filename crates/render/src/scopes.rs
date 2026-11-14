//! Plots the colour scopes of one displayed frame.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Width and height of every plot, in pixels: one cell per 8-bit level.
pub const SCOPE_SIDE: usize = 256;

/// Which scope to plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    /// Luma level (rows, white at the top) against horizontal position (columns).
    Waveform,
    /// Blue difference (columns) against red difference (rows, up) of every pixel.
    Vectorscope,
    /// Count of each red, green and blue level, drawn as three additive channel columns.
    Histogram,
}

impl ScopeKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 3] = [Self::Waveform, Self::Vectorscope, Self::Histogram];

    /// Whether the plot is a single-colour density the caller tints; the histogram carries its
    /// own channel colours.
    pub fn tinted(self) -> bool {
        !matches!(self, Self::Histogram)
    }
}

/// A square RGBA plot, straight alpha, row-major, [`SCOPE_SIDE`] cells a side. Cells with
/// nothing plotted are fully transparent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plot {
    pub rgba: Vec<u8>,
}

/// Plot `kind` for a frame of straight-alpha 8-bit `rgba` pixels, `width` by `height`.
///
/// One pass over the pixels, bounded by the input length; the plot is a fixed 256 KiB.
/// A zero size or a length other than `width * height * 4` is [`Error::Invalid`]. Alpha is
/// ignored: the preview frame is opaque. A frame narrower than 256 pixels leaves the waveform
/// columns it does not reach empty.
pub fn plot(kind: ScopeKind, width: usize, height: usize, rgba: &[u8]) -> Result<Plot> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::Invalid("scope frame size"))?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(Error::Invalid("scope frame size"));
    }
    let pixels = rgba.chunks_exact(4).map(|pixel| match pixel {
        [r, g, b, _] => (*r, *g, *b),
        _ => (0, 0, 0),
    });
    match kind {
        ScopeKind::Waveform => {
            let mut cells = Cells::new()?;
            for (n, (r, g, b)) in pixels.enumerate() {
                let column = (n % width) * SCOPE_SIDE / width;
                cells.bump(row_of(luma(r, g, b)) * SCOPE_SIDE + column);
            }
            cells.density()
        }
        ScopeKind::Vectorscope => {
            let mut cells = Cells::new()?;
            for (r, g, b) in pixels {
                let (cb, cr) = chroma(r, g, b);
                cells.bump(row_of(cr) * SCOPE_SIDE + usize::from(cb));
            }
            cells.density()
        }
        ScopeKind::Histogram => {
            let mut bins = [[0_u32; SCOPE_SIDE]; 3];
            for (r, g, b) in pixels {
                for (bin, level) in bins.iter_mut().zip([r, g, b]) {
                    if let Some(count) = bin.get_mut(usize::from(level)) {
                        *count += 1;
                    }
                }
            }
            histogram(&bins)
        }
    }
}

/// Rec. 709 luma of an 8-bit pixel, integer weights summing to 256.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    let sum = 54 * u32::from(r) + 183 * u32::from(g) + 19 * u32::from(b);
    u8::try_from(sum >> 8).unwrap_or(u8::MAX)
}

/// Blue and red difference of an 8-bit pixel, centred on 128 (grey sits at 128, 128).
fn chroma(r: u8, g: u8, b: u8) -> (u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let cb = 128 + ((-38 * r - 74 * g + 112 * b) >> 8);
    let cr = 128 + ((112 * r - 94 * g - 18 * b) >> 8);
    (clamp_level(cb), clamp_level(cr))
}

fn clamp_level(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or(u8::MAX)
}

/// Plot row of a level: the highest level is the top row.
fn row_of(level: u8) -> usize {
    SCOPE_SIDE - 1 - usize::from(level)
}

struct Cells(Vec<u32>);

impl Cells {
    fn new() -> Result<Self> {
        let mut cells = Vec::new();
        cells.try_reserve_exact(SCOPE_SIDE * SCOPE_SIDE)?;
        cells.resize(SCOPE_SIDE * SCOPE_SIDE, 0);
        Ok(Self(cells))
    }
    fn bump(&mut self, index: usize) {
        if let Some(cell) = self.0.get_mut(index) {
            *cell += 1;
        }
    }
    /// White cells whose alpha follows the square root of the count against the busiest cell,
    /// so a lone pixel stays visible beside a dense one.
    fn density(&self) -> Result<Plot> {
        let peak = self.0.iter().copied().max().unwrap_or(0).max(1);
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(self.0.len() * 4)?;
        for &count in &self.0 {
            let alpha = if count == 0 {
                0
            } else {
                let ratio = f64::from(count) / f64::from(peak);
                // ceil keeps any occupied cell at alpha 1 or more.
                (ratio.sqrt() * 255.0).ceil().clamp(1.0, 255.0) as u8
            };
            rgba.extend_from_slice(&[255, 255, 255, alpha]);
        }
        Ok(Plot { rgba })
    }
}

/// Three channel bars per level, scaled to the tallest bin, each pixel the sum of the
/// channels whose bar reaches it.
fn histogram(bins: &[[u32; SCOPE_SIDE]; 3]) -> Result<Plot> {
    let peak = bins
        .iter()
        .flat_map(|bin| bin.iter().copied())
        .max()
        .unwrap_or(0)
        .max(1);
    let heights: Vec<[usize; 3]> = (0..SCOPE_SIDE)
        .map(|level| {
            let mut out = [0_usize; 3];
            for (height, bin) in out.iter_mut().zip(bins) {
                let count = bin.get(level).copied().unwrap_or(0);
                if count > 0 {
                    let scaled = u64::from(count) * SCOPE_SIDE as u64 / u64::from(peak);
                    *height = usize::try_from(scaled).unwrap_or(SCOPE_SIDE).max(1);
                }
            }
            out
        })
        .collect();
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(SCOPE_SIDE * SCOPE_SIDE * 4)?;
    for row in 0..SCOPE_SIDE {
        // A bar reaches this row when it is at least as tall as the row's distance from the bottom.
        let needed = SCOPE_SIDE - row;
        for column in &heights {
            let mut pixel = [0_u8; 3];
            let mut reached = false;
            for (value, height) in pixel.iter_mut().zip(column) {
                if *height >= needed {
                    *value = 255;
                    reached = true;
                }
            }
            let [r, g, b] = pixel;
            rgba.extend_from_slice(&[r, g, b, if reached { 255 } else { 0 }]);
        }
    }
    Ok(Plot { rgba })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat(width * height)
    }

    fn cell(plot: &Plot, row: usize, column: usize) -> [u8; 4] {
        let at = (row * SCOPE_SIDE + column) * 4;
        [
            plot.rgba[at],
            plot.rgba[at + 1],
            plot.rgba[at + 2],
            plot.rgba[at + 3],
        ]
    }

    #[test]
    fn a_wrong_length_or_empty_frame_is_refused() {
        assert!(plot(ScopeKind::Waveform, 0, 4, &[]).is_err());
        assert!(plot(ScopeKind::Vectorscope, 2, 2, &[0; 15]).is_err());
        assert!(plot(ScopeKind::Histogram, usize::MAX, 2, &[0; 4]).is_err());
    }

    #[test]
    fn a_white_frame_fills_the_top_row_of_the_waveform() {
        let frame = solid(256, 4, [255, 255, 255, 255]);
        let made = plot(ScopeKind::Waveform, 256, 4, &frame).unwrap();
        assert_eq!(made.rgba.len(), SCOPE_SIDE * SCOPE_SIDE * 4);
        for column in 0..SCOPE_SIDE {
            assert_eq!(cell(&made, 0, column), [255, 255, 255, 255]);
            assert_eq!(cell(&made, 1, column)[3], 0);
        }
    }

    #[test]
    fn mid_grey_sits_at_the_vectorscope_centre_and_red_up_and_left() {
        let grey = plot(ScopeKind::Vectorscope, 2, 2, &solid(2, 2, [128, 128, 128, 255])).unwrap();
        // Chroma 128, 128 lands on row 255 - 128 = 127, column 128.
        assert_eq!(cell(&grey, 127, 128)[3], 255);
        let red = plot(ScopeKind::Vectorscope, 2, 2, &solid(2, 2, [255, 0, 0, 255])).unwrap();
        let occupied: Vec<usize> = (0..SCOPE_SIDE * SCOPE_SIDE)
            .filter(|n| red.rgba[n * 4 + 3] > 0)
            .collect();
        assert_eq!(occupied.len(), 1);
        let (row, column) = (occupied[0] / SCOPE_SIDE, occupied[0] % SCOPE_SIDE);
        assert!(row < 127, "red has a red difference above grey");
        assert!(column < 128, "red has a blue difference below grey");
    }

    #[test]
    fn the_histogram_draws_each_channel_at_its_own_level() {
        // Red at 255, green and blue at 0 over the whole frame.
        let made = plot(ScopeKind::Histogram, 2, 2, &solid(2, 2, [255, 0, 0, 255])).unwrap();
        let bottom = SCOPE_SIDE - 1;
        assert_eq!(cell(&made, bottom, 255), [255, 0, 0, 255]);
        assert_eq!(cell(&made, bottom, 0), [0, 255, 255, 255]);
        assert_eq!(cell(&made, bottom, 128)[3], 0);
        assert_eq!(cell(&made, 0, 255), [255, 0, 0, 255]);
    }

    #[test]
    fn only_the_histogram_carries_its_own_colours() {
        assert!(ScopeKind::Waveform.tinted());
        assert!(ScopeKind::Vectorscope.tinted());
        assert!(!ScopeKind::Histogram.tinted());
    }

    #[test]
    fn kinds_round_trip_as_snake_case() {
        let text = serde_json::to_string(&ScopeKind::Vectorscope).unwrap();
        assert_eq!(text, "\"vectorscope\"");
        let back: ScopeKind = serde_json::from_str(&text).unwrap();
        assert_eq!(back, ScopeKind::Vectorscope);
    }
}

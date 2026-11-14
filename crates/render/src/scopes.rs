//! Plots the colour scopes of one displayed frame.

use crate::{Error, Result};

/// Columns of the parade and waveform plots: three channel groups of [`GROUP`] columns.
pub const TRACE_WIDTH: usize = 111;
/// Rows of the parade and waveform plots.
pub const TRACE_HEIGHT: usize = 144;
/// Width and height of the vectorscope plot.
pub const VECTOR_SIDE: usize = 119;
/// Columns of one parade channel.
const GROUP: usize = TRACE_WIDTH / 3;

/// A straight-alpha RGBA plot, row-major. Cells with nothing plotted are fully transparent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plot {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// The three scopes of one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scopes {
    /// Red, green and blue levels side by side, each channel in its own colour.
    pub parade: Plot,
    /// Luma-scale level (rows, high at the top) against horizontal position, each cell in the
    /// mean colour of the pixels it holds.
    pub waveform: Plot,
    /// Blue difference (columns) against red difference (rows, up), each cell in the hue of its
    /// pixels at full brightness.
    pub vectorscope: Plot,
}

/// Plot the parade, waveform and vectorscope of a frame of straight-alpha 8-bit `rgba` pixels,
/// `width` by `height`.
///
/// One pass over the pixels, bounded by the input length; the plots are a fixed size. A zero size
/// or a length other than `width * height * 4` is [`Error::Invalid`]. Alpha is ignored: the
/// preview frame is opaque. A frame narrower than a plot leaves the columns it does not reach
/// empty.
pub fn plot_scopes(width: usize, height: usize, rgba: &[u8]) -> Result<Scopes> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::Invalid("scope frame size"))?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(Error::Invalid("scope frame size"));
    }
    let mut parade = Grid::new(TRACE_WIDTH, TRACE_HEIGHT)?;
    let mut waveform = Grid::new(TRACE_WIDTH, TRACE_HEIGHT)?;
    let mut vectorscope = Grid::new(VECTOR_SIDE, VECTOR_SIDE)?;
    for (n, &[r, g, b, _]) in rgba.as_chunks::<4>().0.iter().enumerate() {
        let x = n % width;
        let within = x * GROUP / width;
        for (channel, (level, colour)) in [(r, [255, 0, 0]), (g, [0, 255, 0]), (b, [0, 0, 255])]
            .into_iter()
            .enumerate()
        {
            parade.add(
                channel * GROUP + within,
                row_of(level, TRACE_HEIGHT),
                colour,
            );
        }
        waveform.add(
            x * TRACE_WIDTH / width,
            row_of(luma(r, g, b), TRACE_HEIGHT),
            [r, g, b],
        );
        let (cb, cr) = chroma(r, g, b);
        vectorscope.add(
            usize::from(cb) * (VECTOR_SIDE - 1) / 255,
            row_of(cr, VECTOR_SIDE),
            [r, g, b],
        );
    }
    Ok(Scopes {
        parade: parade.finish(false)?,
        waveform: waveform.finish(false)?,
        vectorscope: vectorscope.finish(true)?,
    })
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

/// Plot row of an 8-bit level in `rows` rows: the highest level is the top row.
fn row_of(level: u8, rows: usize) -> usize {
    (255 - usize::from(level)) * (rows - 1) / 255
}

/// Pixel counts and colour sums per plot cell.
struct Grid {
    width: usize,
    height: usize,
    count: Vec<u32>,
    sum: Vec<[u64; 3]>,
}

impl Grid {
    fn new(width: usize, height: usize) -> Result<Self> {
        let cells = width * height;
        let mut count = Vec::new();
        count.try_reserve_exact(cells)?;
        count.resize(cells, 0);
        let mut sum = Vec::new();
        sum.try_reserve_exact(cells)?;
        sum.resize(cells, [0; 3]);
        Ok(Self {
            width,
            height,
            count,
            sum,
        })
    }

    fn add(&mut self, column: usize, row: usize, colour: [u8; 3]) {
        let index = row * self.width + column;
        if let (Some(count), Some(sum)) = (self.count.get_mut(index), self.sum.get_mut(index)) {
            *count += 1;
            for (total, value) in sum.iter_mut().zip(colour) {
                *total += u64::from(value);
            }
        }
    }

    /// Cells in the mean colour of their pixels (scaled to full brightness when `bright`), their
    /// alpha following the square root of the count against the busiest cell, so a lone pixel
    /// stays visible beside a dense one.
    fn finish(&self, bright: bool) -> Result<Plot> {
        let peak = self.count.iter().copied().max().unwrap_or(0).max(1);
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(self.count.len() * 4)?;
        for (&count, sum) in self.count.iter().zip(&self.sum) {
            if count == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let mean = sum.map(|total| u8::try_from(total / u64::from(count)).unwrap_or(u8::MAX));
            let top = mean.iter().copied().max().unwrap_or(0);
            let [r, g, b] = if bright && top == 0 {
                [255; 3]
            } else if bright {
                mean.map(|value| {
                    u8::try_from(u32::from(value) * 255 / u32::from(top)).unwrap_or(255)
                })
            } else {
                mean
            };
            let ratio = f64::from(count) / f64::from(peak);
            // ceil keeps any occupied cell at alpha 1 or more.
            let alpha = (ratio.sqrt() * 255.0).ceil().clamp(1.0, 255.0) as u8;
            rgba.extend_from_slice(&[r, g, b, alpha]);
        }
        Ok(Plot {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat(width * height)
    }

    fn cell(plot: &Plot, row: usize, column: usize) -> [u8; 4] {
        let at = (row * plot.width + column) * 4;
        [
            plot.rgba[at],
            plot.rgba[at + 1],
            plot.rgba[at + 2],
            plot.rgba[at + 3],
        ]
    }

    fn occupied(plot: &Plot) -> Vec<(usize, usize)> {
        (0..plot.width * plot.height)
            .filter(|n| plot.rgba[n * 4 + 3] > 0)
            .map(|n| (n / plot.width, n % plot.width))
            .collect()
    }

    #[test]
    fn a_wrong_length_or_empty_frame_is_refused() {
        assert!(plot_scopes(0, 4, &[]).is_err());
        assert!(plot_scopes(2, 2, &[0; 15]).is_err());
        assert!(plot_scopes(usize::MAX, 2, &[0; 4]).is_err());
    }

    #[test]
    fn plots_have_the_measured_sizes() {
        let scopes = plot_scopes(2, 2, &solid(2, 2, [10, 20, 30, 255])).unwrap();
        for (plot, size) in [
            (&scopes.parade, (TRACE_WIDTH, TRACE_HEIGHT)),
            (&scopes.waveform, (TRACE_WIDTH, TRACE_HEIGHT)),
            (&scopes.vectorscope, (VECTOR_SIDE, VECTOR_SIDE)),
        ] {
            assert_eq!((plot.width, plot.height), size);
            assert_eq!(plot.rgba.len(), size.0 * size.1 * 4);
        }
    }

    #[test]
    fn the_parade_draws_red_green_and_blue_in_their_own_thirds() {
        // Red at full, green and blue at nothing: red on the top row of the first third, green
        // and blue on the bottom row of the second and third.
        let frame = solid(111, 2, [255, 0, 0, 255]);
        let scopes = plot_scopes(111, 2, &frame).unwrap();
        let bottom = TRACE_HEIGHT - 1;
        assert_eq!(cell(&scopes.parade, 0, 0), [255, 0, 0, 255]);
        assert_eq!(cell(&scopes.parade, bottom, 37), [0, 255, 0, 255]);
        assert_eq!(cell(&scopes.parade, bottom, 74), [0, 0, 255, 255]);
        assert_eq!(cell(&scopes.parade, 0, 37)[3], 0);
    }

    #[test]
    fn a_white_frame_fills_the_top_row_of_the_waveform_in_white() {
        let frame = solid(111, 4, [255, 255, 255, 255]);
        let scopes = plot_scopes(111, 4, &frame).unwrap();
        for column in 0..TRACE_WIDTH {
            assert_eq!(cell(&scopes.waveform, 0, column), [255, 255, 255, 255]);
            assert_eq!(cell(&scopes.waveform, 1, column)[3], 0);
        }
    }

    #[test]
    fn grey_sits_at_the_vectorscope_centre_and_red_up_and_left_in_its_own_hue() {
        let grey = plot_scopes(2, 2, &solid(2, 2, [128, 128, 128, 255])).unwrap();
        let centre = VECTOR_SIDE / 2;
        assert_eq!(occupied(&grey.vectorscope), vec![(centre - 1, centre)]);
        let red = plot_scopes(2, 2, &solid(2, 2, [255, 0, 0, 255])).unwrap();
        let cells = occupied(&red.vectorscope);
        assert_eq!(cells.len(), 1);
        let (row, column) = cells[0];
        assert!(row < centre - 1, "red has a red difference above grey");
        assert!(column < centre, "red has a blue difference below grey");
        assert_eq!(cell(&red.vectorscope, row, column), [255, 0, 0, 255]);
    }
}

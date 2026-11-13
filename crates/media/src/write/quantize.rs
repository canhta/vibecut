//! Reduces one RGBA frame to an indexed palette frame deterministically.

use super::gif::GifDither;

/// Colour cells per channel are 5 bits wide: 32 768 histogram cells.
const CELLS: usize = 1 << 15;
const UNSET: u16 = u16::MAX;
/// 4x4 ordered-dither thresholds.
const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// One frame as palette indices plus the colours they name.
pub(super) struct Indexed {
    pub(super) indices: Vec<u8>,
    /// Opaque `0xFFRRGGBB` entries; unused trailing entries are opaque black.
    pub(super) palette: [u32; 256],
}

struct Cell {
    key: u16,
    count: u32,
    sum: [u64; 3],
}
impl Cell {
    fn channel(&self, channel: usize) -> u16 {
        (self.key >> (10 - 5 * channel)) & 31
    }
    fn mean(&self) -> [i32; 3] {
        let count = u64::from(self.count).max(1);
        self.sum.map(|total| ((total + count / 2) / count) as i32)
    }
}

/// Median-cut palette of at most `max_colors` entries from a 5-bit-per-channel
/// histogram. Frames with no more distinct cells than `max_colors` keep every
/// cell exactly and are never dithered. Alpha is ignored. Equal input gives
/// equal output on every host.
pub(super) fn quantize(rgba: &[u8], width: usize, max_colors: usize, dither: GifDither) -> Indexed {
    let mut cells: Vec<Cell> = Vec::new();
    let mut slot = vec![UNSET; CELLS];
    for pixel in rgba.chunks_exact(4) {
        let key = cell_key(pixel);
        let Some(entry) = slot.get_mut(key) else {
            continue;
        };
        if *entry == UNSET {
            *entry = cells.len() as u16;
            cells.push(Cell {
                key: key as u16,
                count: 0,
                sum: [0; 3],
            });
        }
        if let Some(cell) = cells.get_mut(usize::from(*entry)) {
            cell.count += 1;
            for (total, value) in cell.sum.iter_mut().zip(pixel) {
                *total += u64::from(*value);
            }
        }
    }
    let exact = cells.len() <= max_colors;
    let boxes = split(cells, max_colors);
    let mut means = Vec::with_capacity(boxes.len());
    let mut lut = vec![UNSET; CELLS];
    for cell_box in &boxes {
        let mut count = 0u64;
        let mut sum = [0u64; 3];
        for cell in cell_box {
            count += u64::from(cell.count);
            for (total, part) in sum.iter_mut().zip(cell.sum) {
                *total += part;
            }
        }
        let count = count.max(1);
        means.push(sum.map(|total| ((total + count / 2) / count) as i32));
    }
    // Every populated cell maps to the nearest palette entry of its own mean.
    for (index, cell_box) in boxes.iter().enumerate() {
        for cell in cell_box {
            if let Some(entry) = lut.get_mut(usize::from(cell.key)) {
                *entry = if exact {
                    index as u16
                } else {
                    nearest(&means, cell.mean())
                };
            }
        }
    }
    let mut palette = [0xFF00_0000u32; 256];
    for (entry, mean) in palette.iter_mut().zip(&means) {
        let [r, g, b] = mean.map(|v| v.clamp(0, 255) as u32);
        *entry = 0xFF00_0000 | (r << 16) | (g << 8) | b;
    }
    let width = width.max(1);
    let dithered = !exact && dither == GifDither::Bayer;
    let mut indices = Vec::with_capacity(rgba.len() / 4);
    for (n, pixel) in rgba.chunks_exact(4).enumerate() {
        let mut key = cell_key(pixel);
        if dithered {
            let (x, y) = (n % width, n / width);
            let threshold = BAYER.get((y % 4) * 4 + x % 4).copied().map_or(0, i32::from);
            let [r, g, b] = [0usize, 1, 2].map(|c| {
                let value = pixel.get(c).copied().map_or(0, i32::from) + threshold - 8;
                (value.clamp(0, 255) as usize) >> 3
            });
            key = (r << 10) | (g << 5) | b;
        }
        let index = match lut.get_mut(key) {
            Some(entry) => {
                if *entry == UNSET {
                    // Cell not seen in the histogram (dither moved the pixel).
                    let centre =
                        [key >> 10, (key >> 5) & 31, key & 31].map(|v| ((v << 3) | 4) as i32);
                    *entry = nearest(&means, centre);
                }
                *entry
            }
            None => 0,
        };
        indices.push(index as u8);
    }
    Indexed { indices, palette }
}

fn cell_key(pixel: &[u8]) -> usize {
    let channel = |i: usize| usize::from(pixel.get(i).copied().unwrap_or(0) >> 3);
    (channel(0) << 10) | (channel(1) << 5) | channel(2)
}

fn nearest(means: &[[i32; 3]], target: [i32; 3]) -> u16 {
    let mut best = (i64::MAX, 0u16);
    for (index, mean) in means.iter().enumerate() {
        let distance: i64 = mean
            .iter()
            .zip(target)
            .map(|(a, b)| i64::from(a - b) * i64::from(a - b))
            .sum();
        if distance < best.0 {
            best = (distance, index as u16);
        }
    }
    best.1
}

/// Median cut: repeatedly split the most populated multi-cell box along its
/// widest channel at the population median.
fn split(cells: Vec<Cell>, max_colors: usize) -> Vec<Vec<Cell>> {
    let mut boxes = vec![cells];
    while boxes.len() < max_colors {
        let Some(target) = boxes
            .iter()
            .enumerate()
            .filter(|(_, cell_box)| cell_box.len() >= 2)
            .max_by_key(|(index, cell_box)| {
                (
                    cell_box.iter().map(|c| u64::from(c.count)).sum::<u64>(),
                    std::cmp::Reverse(*index),
                )
            })
            .map(|(index, _)| index)
        else {
            break;
        };
        let mut cell_box = boxes.swap_remove(target);
        let channel = (0..3)
            .map(|c| {
                let low = cell_box
                    .iter()
                    .map(|cell| cell.channel(c))
                    .min()
                    .unwrap_or(0);
                let high = cell_box
                    .iter()
                    .map(|cell| cell.channel(c))
                    .max()
                    .unwrap_or(0);
                (high - low, std::cmp::Reverse(c))
            })
            .max()
            .map_or(0, |(_, std::cmp::Reverse(c))| c);
        cell_box.sort_by_key(|cell| (cell.channel(channel), cell.key));
        let total: u64 = cell_box.iter().map(|c| u64::from(c.count)).sum();
        let mut seen = 0u64;
        let mut middle = cell_box.len() - 1;
        for (index, cell) in cell_box.iter().enumerate() {
            seen += u64::from(cell.count);
            if seen * 2 >= total {
                middle = index + 1;
                break;
            }
        }
        let middle = middle.clamp(1, cell_box.len() - 1);
        let upper = cell_box.split_off(middle);
        boxes.push(cell_box);
        boxes.push(upper);
    }
    boxes
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Vec<u8> {
        (0..width * height)
            .flat_map(|n| {
                let (x, y) = (n % width, n / width);
                [
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    ((x + y) % 256) as u8,
                    255,
                ]
            })
            .collect()
    }

    #[test]
    fn palette_is_bounded_deterministic_and_exact_for_flat_colours() {
        let gradient = gradient(64, 48);
        for (max_colors, dither) in [
            (2, GifDither::None),
            (16, GifDither::Bayer),
            (256, GifDither::Bayer),
        ] {
            let first = quantize(&gradient, 64, max_colors, dither);
            let again = quantize(&gradient, 64, max_colors, dither);
            assert_eq!(first.indices, again.indices);
            assert_eq!(first.palette, again.palette);
            assert_eq!(first.indices.len(), 64 * 48);
            let used: std::collections::BTreeSet<_> = first.indices.iter().collect();
            assert!(used.len() <= max_colors, "{max_colors}: {}", used.len());
        }
        let flat: Vec<u8> = [10u8, 200, 30, 255, 250, 5, 99, 255]
            .iter()
            .copied()
            .cycle()
            .take(8 * 4 * 8)
            .collect();
        let indexed = quantize(&flat, 8, 4, GifDither::Bayer);
        assert_eq!(
            indexed
                .indices
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
        let colours: Vec<u32> = indexed
            .indices
            .iter()
            .map(|&i| indexed.palette[usize::from(i)])
            .collect();
        assert_eq!(colours[0], 0xFF0A_C81E);
        assert_eq!(colours[1], 0xFFFA_0563);
        assert!(
            colours
                .chunks(2)
                .all(|pair| pair == [0xFF0A_C81E, 0xFFFA_0563])
        );
    }
}
